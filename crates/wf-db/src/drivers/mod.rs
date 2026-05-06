pub mod my;
pub mod pg;
pub mod sqlite;

use std::collections::HashMap;

use crate::models::{ColumnInfo, DbMetadata, TableInfo};

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Returns `true` if `sql` is expected to return rows (SELECT, WITH, PRAGMA,
/// SHOW, EXPLAIN, DESCRIBE, VALUES, TABLE), and `false` for DML / DDL
/// statements (INSERT, UPDATE, DELETE, CREATE, DROP, ALTER, TRUNCATE, …).
///
/// Only the leading keyword is inspected, so the check is intentionally
/// simple. Edge-cases like a CTE starting with `WITH` are handled correctly
/// because `WITH … SELECT` is row-returning by its first keyword.
pub(super) fn is_row_returning(sql: &str) -> bool {
    let keyword: String = sql
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect();
    matches!(
        keyword.to_ascii_uppercase().as_str(),
        "SELECT"
            | "WITH"
            | "PRAGMA"
            | "SHOW"
            | "EXPLAIN"
            | "DESCRIBE"
            | "DESC"
            | "VALUES"
            | "TABLE"
    )
}

// ---------------------------------------------------------------------------
// impl_execute! macro
// ---------------------------------------------------------------------------

/// Expand a standard `execute` function for the given pool type.
///
/// Relies on `cell_to_string(row, i) -> Option<String>` being defined in the
/// calling module — each driver supplies its own driver-specific version.
macro_rules! impl_execute {
    ($pool_ty:ty) => {
        /// Execute `sql` against `pool` and return a [`crate::models::QueryResult`].
        ///
        /// - **SELECT / row-returning statements**: columns and rows are populated.
        /// - **DML / DDL statements**: rows are empty; `row_count` = `rows_affected()`.
        /// - **NULL values** map to `None`.
        /// - `execution_time_ms` is measured with [`std::time::Instant`].
        pub async fn execute(
            pool: &$pool_ty,
            sql: &str,
        ) -> Result<crate::models::QueryResult, crate::error::DbError> {
            use sqlx::{Column as _, Row as _};
            let started = ::std::time::Instant::now();

            if super::is_row_returning(sql) {
                let rows = sqlx::query(sql)
                    .fetch_all(pool)
                    .await
                    .map_err(crate::error::DbError::from)?;

                let columns: Vec<String> = rows
                    .first()
                    .map(|r| r.columns().iter().map(|c| c.name().to_string()).collect())
                    .unwrap_or_default();

                let data: Vec<Vec<Option<String>>> = rows
                    .iter()
                    .map(|row| (0..row.len()).map(|i| cell_to_string(row, i)).collect())
                    .collect();

                let row_count = data.len();
                Ok(crate::models::QueryResult {
                    columns,
                    rows: data,
                    row_count,
                    execution_time_ms: started.elapsed().as_millis(),
                })
            } else {
                let result = sqlx::query(sql)
                    .execute(pool)
                    .await
                    .map_err(crate::error::DbError::from)?;

                Ok(crate::models::QueryResult {
                    columns: vec![],
                    rows: vec![],
                    row_count: result.rows_affected() as usize,
                    execution_time_ms: started.elapsed().as_millis(),
                })
            }
        }
    };
}
pub(crate) use impl_execute;

// ---------------------------------------------------------------------------
// MetadataAccumulator
// ---------------------------------------------------------------------------

/// Builder that accumulates schema metadata during `fetch_metadata`.
///
/// **PG / MySQL**: call [`add_column`] for each column row first, then
/// [`add_table`] / [`add_view`] per object row (each drains the column map
/// for that name).  Call [`set_stored_procs`] and [`set_indexes`] for the
/// remaining result sets, then [`build`].
///
/// **SQLite**: call [`push_table`] / [`push_view`] directly (columns come
/// from `PRAGMA table_info`, one round-trip per object).  Call [`set_indexes`]
/// then [`build`].
pub(super) struct MetadataAccumulator {
    col_map: HashMap<String, Vec<ColumnInfo>>,
    tables: Vec<TableInfo>,
    views: Vec<TableInfo>,
    stored_procs: Vec<String>,
    indexes: Vec<String>,
}

impl MetadataAccumulator {
    pub fn new() -> Self {
        Self {
            col_map: HashMap::new(),
            tables: Vec::new(),
            views: Vec::new(),
            stored_procs: Vec::new(),
            indexes: Vec::new(),
        }
    }

    /// Record a column belonging to `table`.  Call before [`add_table`] /
    /// [`add_view`] for the same table so the map is populated when those
    /// methods drain it.
    pub fn add_column(&mut self, table: String, col: ColumnInfo) {
        self.col_map.entry(table).or_default().push(col);
    }

    /// Append a table entry, draining its columns from the internal map.
    pub fn add_table(&mut self, name: String) {
        let columns = self.col_map.remove(&name).unwrap_or_default();
        self.tables.push(TableInfo { name, columns });
    }

    /// Append a view entry, draining its columns from the internal map.
    pub fn add_view(&mut self, name: String) {
        let columns = self.col_map.remove(&name).unwrap_or_default();
        self.views.push(TableInfo { name, columns });
    }

    /// Append a fully-constructed table (for SQLite, which fetches columns via PRAGMA).
    pub fn push_table(&mut self, info: TableInfo) {
        self.tables.push(info);
    }

    /// Append a fully-constructed view (for SQLite).
    pub fn push_view(&mut self, info: TableInfo) {
        self.views.push(info);
    }

    pub fn set_stored_procs(&mut self, procs: Vec<String>) {
        self.stored_procs = procs;
    }

    pub fn set_indexes(&mut self, indexes: Vec<String>) {
        self.indexes = indexes;
    }

    /// Consume the accumulator and return the final [`DbMetadata`].
    pub fn build(self) -> DbMetadata {
        DbMetadata {
            tables: self.tables,
            views: self.views,
            stored_procs: self.stored_procs,
            indexes: self.indexes,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_row_returning ─────────────────────────────────────────────────────

    #[test]
    fn is_row_returning_should_return_true_for_select() {
        assert!(is_row_returning("SELECT 1"));
        assert!(is_row_returning("  select * from t"));
        assert!(is_row_returning("WITH cte AS (SELECT 1) SELECT * FROM cte"));
    }

    #[test]
    fn is_row_returning_should_return_false_for_dml() {
        assert!(!is_row_returning("INSERT INTO t VALUES (1)"));
        assert!(!is_row_returning("UPDATE t SET x = 1"));
        assert!(!is_row_returning("DELETE FROM t"));
        assert!(!is_row_returning("CREATE TABLE t (id INTEGER)"));
        assert!(!is_row_returning("DROP TABLE t"));
        assert!(!is_row_returning("ALTER TABLE t ADD COLUMN x TEXT"));
    }

    // ── MetadataAccumulator ───────────────────────────────────────────────────

    #[test]
    fn metadata_accumulator_build_should_return_empty_metadata_by_default() {
        let acc = MetadataAccumulator::new();
        let meta = acc.build();
        assert!(meta.tables.is_empty());
        assert!(meta.views.is_empty());
        assert!(meta.stored_procs.is_empty());
        assert!(meta.indexes.is_empty());
    }

    #[test]
    fn metadata_accumulator_should_assemble_tables_with_columns() {
        let mut acc = MetadataAccumulator::new();
        acc.add_column(
            "users".to_string(),
            ColumnInfo {
                name: "id".to_string(),
                data_type: "INTEGER".to_string(),
                nullable: false,
            },
        );
        acc.add_column(
            "users".to_string(),
            ColumnInfo {
                name: "name".to_string(),
                data_type: "TEXT".to_string(),
                nullable: true,
            },
        );
        acc.add_table("users".to_string());

        let meta = acc.build();
        assert_eq!(meta.tables.len(), 1);
        assert_eq!(meta.tables[0].name, "users");
        assert_eq!(meta.tables[0].columns.len(), 2);
        assert_eq!(meta.tables[0].columns[0].name, "id");
        assert_eq!(meta.tables[0].columns[1].name, "name");
    }

    #[test]
    fn metadata_accumulator_should_assemble_views_with_columns() {
        let mut acc = MetadataAccumulator::new();
        acc.add_column(
            "v".to_string(),
            ColumnInfo {
                name: "x".to_string(),
                data_type: "INT".to_string(),
                nullable: true,
            },
        );
        acc.add_view("v".to_string());

        let meta = acc.build();
        assert_eq!(meta.views.len(), 1);
        assert_eq!(meta.views[0].name, "v");
        assert_eq!(meta.views[0].columns.len(), 1);
        assert_eq!(meta.views[0].columns[0].name, "x");
    }

    #[test]
    fn metadata_accumulator_push_table_should_bypass_column_map() {
        let mut acc = MetadataAccumulator::new();
        acc.push_table(TableInfo {
            name: "t".to_string(),
            columns: vec![ColumnInfo {
                name: "id".to_string(),
                data_type: "INTEGER".to_string(),
                nullable: false,
            }],
        });

        let meta = acc.build();
        assert_eq!(meta.tables.len(), 1);
        assert_eq!(meta.tables[0].name, "t");
        assert_eq!(meta.tables[0].columns.len(), 1);
    }

    #[test]
    fn metadata_accumulator_should_store_procs_and_indexes() {
        let mut acc = MetadataAccumulator::new();
        acc.set_stored_procs(vec!["proc_a".to_string(), "proc_b".to_string()]);
        acc.set_indexes(vec!["idx_1".to_string()]);

        let meta = acc.build();
        assert_eq!(meta.stored_procs, vec!["proc_a", "proc_b"]);
        assert_eq!(meta.indexes, vec!["idx_1"]);
    }

    #[test]
    fn metadata_accumulator_add_table_should_use_empty_columns_when_none_recorded() {
        let mut acc = MetadataAccumulator::new();
        acc.add_table("orphan".to_string()); // no columns added first

        let meta = acc.build();
        assert_eq!(meta.tables.len(), 1);
        assert!(meta.tables[0].columns.is_empty());
    }
}
