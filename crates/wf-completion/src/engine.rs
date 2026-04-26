//! Completion candidate generation.
//!
//! [`CompletionEngine::complete`] maps a [`CompletionContext`] + [`DbMetadata`] + prefix
//! string to a filtered list of [`CompletionItem`] candidates.

use wf_db::models::DbMetadata;

use crate::parser::CompletionContext;
use crate::{CompletionItem, CompletionKind};

// Value literal candidates shown after a comparison operator (=, <, >).
const VALUE_CANDIDATES: &[(&str, i32)] = &[
    ("''", 1), // cursor_offset 1 → places cursor between quotes
    ("NULL", 0),
    ("TRUE", 0),
    ("FALSE", 0),
];

// ── SQL keyword table ─────────────────────────────────────────────────────────

const SQL_KEYWORDS: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "JOIN",
    "INNER JOIN",
    "LEFT JOIN",
    "RIGHT JOIN",
    "FULL OUTER JOIN",
    "CROSS JOIN",
    "ON",
    "AS",
    "GROUP BY",
    "ORDER BY",
    "HAVING",
    "LIMIT",
    "OFFSET",
    "INSERT INTO",
    "VALUES",
    "UPDATE",
    "SET",
    "DELETE FROM",
    "CREATE TABLE",
    "DROP TABLE",
    "ALTER TABLE",
    "ADD COLUMN",
    "CREATE INDEX",
    "DROP INDEX",
    "CREATE VIEW",
    "DROP VIEW",
    "DISTINCT",
    "ALL",
    "UNION",
    "UNION ALL",
    "EXCEPT",
    "INTERSECT",
    "AND",
    "OR",
    "NOT",
    "IN",
    "NOT IN",
    "EXISTS",
    "NOT EXISTS",
    "LIKE",
    "ILIKE",
    "BETWEEN",
    "IS NULL",
    "IS NOT NULL",
    "TRUE",
    "FALSE",
    "NULL",
    "CASE",
    "WHEN",
    "THEN",
    "ELSE",
    "END",
    "ASC",
    "DESC",
    "WITH",
];

// ── Public API ────────────────────────────────────────────────────────────────

/// Generates completion candidates from a context, metadata, and typed prefix.
pub struct CompletionEngine;

impl CompletionEngine {
    /// Return all [`CompletionItem`] candidates matching `prefix` for the given `context`.
    ///
    /// Matching is case-insensitive.  Returns an empty vec for [`CompletionContext::None`].
    pub fn complete(
        context: CompletionContext,
        metadata: &DbMetadata,
        prefix: &str,
    ) -> Vec<CompletionItem> {
        let prefix_upper = prefix.to_ascii_uppercase();
        match context {
            CompletionContext::Keyword => {
                if prefix_upper.is_empty() {
                    // Don't flood the popup when the cursor is at an empty position
                    // (e.g. after typing "SELECT ").  The user must type at least one
                    // character before keyword suggestions appear.
                    return vec![];
                }
                // Exclude keywords the user has already typed in full (e.g. if prefix
                // is "SELECT", omit SELECT itself — but keep "UNION ALL" when prefix
                // is "UNION").
                keyword_candidates(&prefix_upper)
                    .into_iter()
                    .filter(|item| item.label.to_ascii_uppercase() != prefix_upper)
                    .collect()
            }
            CompletionContext::TableName => {
                let tables: Vec<_> = table_candidates(metadata, &prefix_upper)
                    .into_iter()
                    .filter(|item| item.label.to_ascii_uppercase() != prefix_upper)
                    .collect();
                if tables.is_empty() && !prefix_upper.is_empty() {
                    // No table matched the prefix — the user is likely typing a
                    // keyword (e.g. "WHERE" after "FROM users wh").  Fall back to
                    // keyword suggestions so structure keywords always surface.
                    keyword_candidates(&prefix_upper)
                        .into_iter()
                        .filter(|item| item.label.to_ascii_uppercase() != prefix_upper)
                        .collect()
                } else {
                    tables
                }
            }
            CompletionContext::ColumnName { table } => {
                // Exclude items the user has already typed in full so the popup
                // closes automatically once a word is complete.
                let cols: Vec<_> = column_candidates(metadata, table.as_deref(), &prefix_upper)
                    .into_iter()
                    .filter(|item| item.label.to_ascii_uppercase() != prefix_upper)
                    .collect();
                if cols.is_empty() && !prefix_upper.is_empty() {
                    // No column matched the prefix — fall back to keywords so the
                    // user can continue structuring the query (AND, OR, ORDER BY …).
                    keyword_candidates(&prefix_upper)
                        .into_iter()
                        .filter(|item| item.label.to_ascii_uppercase() != prefix_upper)
                        .collect()
                } else {
                    cols
                }
            }
            CompletionContext::NextClause => next_clause_candidates(),
            CompletionContext::JoinOn => join_on_candidates(),
            CompletionContext::Operator => operator_candidates(),
            CompletionContext::JoinConditionTable { tables } => tables
                .iter()
                .filter(|name| name.to_ascii_uppercase().starts_with(&prefix_upper))
                .map(|name| CompletionItem {
                    label: name.clone(),
                    kind: CompletionKind::Table,
                    insert_text: name.clone(),
                    cursor_offset: 0,
                    detail: None,
                    table_name: None,
                })
                .collect(),
            CompletionContext::ValueExpected => value_candidates(),
            CompletionContext::None => vec![],
        }
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn next_clause_candidates() -> Vec<CompletionItem> {
    const NEXT_CLAUSES: &[&str] = &[
        "WHERE",
        "JOIN",
        "INNER JOIN",
        "LEFT JOIN",
        "RIGHT JOIN",
        "FULL OUTER JOIN",
        "ORDER BY",
        "GROUP BY",
        "HAVING",
        "LIMIT",
        "OFFSET",
        "ON",
        "UNION",
        "UNION ALL",
    ];
    NEXT_CLAUSES
        .iter()
        .map(|&kw| CompletionItem {
            label: kw.to_string(),
            kind: CompletionKind::Keyword,
            insert_text: kw.to_string(),
            cursor_offset: 0,
            detail: None,
            table_name: None,
        })
        .collect()
}

fn join_on_candidates() -> Vec<CompletionItem> {
    vec![CompletionItem {
        label: "ON".to_string(),
        kind: CompletionKind::Keyword,
        insert_text: "ON".to_string(),
        cursor_offset: 0,
        detail: None,
        table_name: None,
    }]
}

fn operator_candidates() -> Vec<CompletionItem> {
    const OPS: &[&str] = &[
        "=",
        "!=",
        "<>",
        "<",
        ">",
        "<=",
        ">=",
        "IS NULL",
        "IS NOT NULL",
        "IN",
        "NOT IN",
        "LIKE",
        "ILIKE",
        "BETWEEN",
    ];
    OPS.iter()
        .map(|&op| CompletionItem {
            label: op.to_string(),
            kind: CompletionKind::Operator,
            insert_text: op.to_string(),
            cursor_offset: 0,
            detail: None,
            table_name: None,
        })
        .collect()
}

fn value_candidates() -> Vec<CompletionItem> {
    VALUE_CANDIDATES
        .iter()
        .map(|&(val, offset)| CompletionItem {
            label: val.to_string(),
            kind: CompletionKind::Keyword,
            insert_text: val.to_string(),
            cursor_offset: offset,
            detail: None,
            table_name: None,
        })
        .collect()
}

fn keyword_candidates(prefix_upper: &str) -> Vec<CompletionItem> {
    SQL_KEYWORDS
        .iter()
        .filter(|&&kw| kw.starts_with(prefix_upper))
        .map(|&kw| CompletionItem {
            label: kw.to_string(),
            kind: CompletionKind::Keyword,
            insert_text: kw.to_string(),
            cursor_offset: 0,
            detail: None,
            table_name: None,
        })
        .collect()
}

fn table_candidates(metadata: &DbMetadata, prefix_upper: &str) -> Vec<CompletionItem> {
    let tables = metadata.tables.iter().map(|t| (t, CompletionKind::Table));
    let views = metadata.views.iter().map(|v| (v, CompletionKind::View));

    tables
        .chain(views)
        .filter(|(t, _)| t.name.to_ascii_uppercase().starts_with(prefix_upper))
        .map(|(t, kind)| CompletionItem {
            label: t.name.clone(),
            kind,
            insert_text: t.name.clone(),
            cursor_offset: 0,
            detail: None,
            table_name: None,
        })
        .collect()
}

fn column_candidates(
    metadata: &DbMetadata,
    table: Option<&str>,
    prefix_upper: &str,
) -> Vec<CompletionItem> {
    match table {
        Some(name) => {
            let name_upper = name.to_ascii_uppercase();
            metadata
                .tables
                .iter()
                .chain(metadata.views.iter())
                .filter(|t| t.name.to_ascii_uppercase() == name_upper)
                .flat_map(|t| {
                    let tname = t.name.clone();
                    t.columns.iter().map(move |c| (c, tname.clone()))
                })
                .filter(|(c, _)| c.name.to_ascii_uppercase().starts_with(prefix_upper))
                .map(|(c, tname)| CompletionItem {
                    label: c.name.clone(),
                    kind: CompletionKind::Column,
                    insert_text: c.name.clone(),
                    cursor_offset: 0,
                    detail: Some(c.data_type.clone()),
                    table_name: Some(tname),
                })
                .collect()
        }
        None => {
            // Collect ALL (col, table) pairs to determine global ambiguity counts.
            let all_pairs: Vec<(&wf_db::models::ColumnInfo, &str)> = metadata
                .tables
                .iter()
                .chain(metadata.views.iter())
                .flat_map(|t| t.columns.iter().map(move |c| (c, t.name.as_str())))
                .collect();

            // Count how many tables each column name appears in (uppercase key).
            let mut tables_per_col: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();
            for (c, _) in &all_pairs {
                *tables_per_col
                    .entry(c.name.to_ascii_uppercase())
                    .or_insert(0) += 1;
            }

            // Filter and build items.
            //
            // Two matching modes:
            //  1. Standard: column name starts with `prefix`  (e.g. "na" → "name")
            //  2. Extended: for ambiguous columns only — prefix starts with the column
            //     name and the remainder matches the table name prefix.
            //     This lets users narrow disambiguation by continuing to type the table
            //     name directly (e.g. "nameuse" → shows only "name (users)").
            all_pairs
                .into_iter()
                .filter(|(c, tname)| {
                    let col_upper = c.name.to_ascii_uppercase();
                    // Standard match: column name starts with prefix.
                    if col_upper.starts_with(prefix_upper) {
                        return true;
                    }
                    // Extended match: only for ambiguous columns.
                    // prefix must start with the column name; the trailing part of
                    // the prefix must match the beginning of the table name.
                    let ambiguous = *tables_per_col.get(&col_upper).unwrap_or(&0) > 1;
                    if ambiguous
                        && !prefix_upper.is_empty()
                        && prefix_upper.starts_with(col_upper.as_str())
                    {
                        // Trim any leading whitespace so "name u" (space-separated)
                        // matches the same way as "nameu" (no-space).
                        let rest = prefix_upper[col_upper.len()..].trim_start_matches(' ');
                        return tname.to_ascii_uppercase().starts_with(rest);
                    }
                    false
                })
                .map(|(c, tname)| {
                    let ambiguous = *tables_per_col
                        .get(&c.name.to_ascii_uppercase())
                        .unwrap_or(&0)
                        > 1;
                    let label = if ambiguous {
                        format!("{} ({})", c.name, tname)
                    } else {
                        c.name.clone()
                    };
                    CompletionItem {
                        label,
                        kind: CompletionKind::Column,
                        insert_text: c.name.clone(),
                        cursor_offset: 0,
                        detail: Some(c.data_type.clone()),
                        table_name: Some(tname.to_string()),
                    }
                })
                .collect()
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use wf_db::models::{ColumnInfo, TableInfo};

    fn make_metadata() -> DbMetadata {
        DbMetadata {
            tables: vec![
                TableInfo {
                    name: "users".to_string(),
                    columns: vec![
                        ColumnInfo {
                            name: "id".to_string(),
                            data_type: "integer".to_string(),
                            nullable: false,
                        },
                        ColumnInfo {
                            name: "email".to_string(),
                            data_type: "varchar".to_string(),
                            nullable: false,
                        },
                    ],
                },
                TableInfo {
                    name: "orders".to_string(),
                    columns: vec![
                        ColumnInfo {
                            name: "order_id".to_string(),
                            data_type: "integer".to_string(),
                            nullable: false,
                        },
                        ColumnInfo {
                            name: "total".to_string(),
                            data_type: "numeric".to_string(),
                            nullable: true,
                        },
                    ],
                },
            ],
            views: vec![TableInfo {
                name: "active_users".to_string(),
                columns: vec![ColumnInfo {
                    name: "user_id".to_string(),
                    data_type: "integer".to_string(),
                    nullable: false,
                }],
            }],
            stored_procs: vec![],
            indexes: vec![],
        }
    }

    #[test]
    fn complete_should_return_keyword_candidates_filtered_by_prefix() {
        let meta = DbMetadata::default();
        let items = CompletionEngine::complete(CompletionContext::Keyword, &meta, "sel");
        let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(
            labels.contains(&"SELECT"),
            "expected SELECT in {:?}",
            labels
        );
        assert!(items.iter().all(|i| i.kind == CompletionKind::Keyword));
    }

    #[test]
    fn complete_should_return_empty_for_keyword_with_empty_prefix() {
        let meta = DbMetadata::default();
        let items = CompletionEngine::complete(CompletionContext::Keyword, &meta, "");
        assert!(
            items.is_empty(),
            "expected no keyword suggestions for empty prefix, got {}",
            items.len()
        );
    }

    #[test]
    fn complete_should_return_table_and_view_names_for_table_name_context() {
        let meta = make_metadata();
        let items = CompletionEngine::complete(CompletionContext::TableName, &meta, "");
        let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"users"));
        assert!(labels.contains(&"orders"));
        assert!(labels.contains(&"active_users"));
    }

    #[test]
    fn complete_should_filter_table_names_by_prefix() {
        let meta = make_metadata();
        let items = CompletionEngine::complete(CompletionContext::TableName, &meta, "ord");
        let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, vec!["orders"]);
    }

    #[test]
    fn complete_should_return_columns_for_specific_table() {
        let meta = make_metadata();
        let ctx = CompletionContext::ColumnName {
            table: Some("users".to_string()),
        };
        let items = CompletionEngine::complete(ctx, &meta, "");
        let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"id"));
        assert!(labels.contains(&"email"));
        assert!(
            !labels.contains(&"order_id"),
            "should not include orders columns"
        );
    }

    #[test]
    fn complete_should_return_all_columns_when_table_is_none() {
        let meta = make_metadata();
        let ctx = CompletionContext::ColumnName { table: None };
        let items = CompletionEngine::complete(ctx, &meta, "");
        let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"id"));
        assert!(labels.contains(&"email"));
        assert!(labels.contains(&"order_id"));
        assert!(labels.contains(&"total"));
        assert!(labels.contains(&"user_id"));
    }

    #[test]
    fn complete_should_fall_back_to_keywords_when_no_table_matches_prefix() {
        let meta = make_metadata();
        // "wh" doesn't match any table/view name → falls back to keywords (WHERE, WITH)
        let items = CompletionEngine::complete(CompletionContext::TableName, &meta, "wh");
        let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"WHERE"), "expected WHERE in {labels:?}");
        assert!(items.iter().all(|i| i.kind == CompletionKind::Keyword));
    }

    #[test]
    fn complete_should_fall_back_to_keywords_when_no_column_matches_prefix() {
        let meta = make_metadata();
        // users columns are "id" and "email"; "wh" matches neither → falls back to keywords
        let ctx = CompletionContext::ColumnName {
            table: Some("users".to_string()),
        };
        let items = CompletionEngine::complete(ctx, &meta, "wh");
        let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"WHERE"), "expected WHERE in {labels:?}");
        assert!(items.iter().all(|i| i.kind == CompletionKind::Keyword));
    }

    #[test]
    fn complete_should_prefer_table_candidates_over_keyword_fallback() {
        let meta = make_metadata();
        // "us" matches table "users" → table candidates returned, no keyword fallback
        let items = CompletionEngine::complete(CompletionContext::TableName, &meta, "us");
        let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, vec!["users"]);
        assert!(items.iter().all(|i| i.kind == CompletionKind::Table));
    }

    #[test]
    fn complete_should_return_on_for_join_on_context() {
        let meta = DbMetadata::default();
        let items = CompletionEngine::complete(CompletionContext::JoinOn, &meta, "");
        let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, vec!["ON"]);
        assert!(items.iter().all(|i| i.kind == CompletionKind::Keyword));
    }

    #[test]
    fn complete_should_return_comparison_operators_for_operator_context() {
        let meta = DbMetadata::default();
        let items = CompletionEngine::complete(CompletionContext::Operator, &meta, "");
        let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"="), "expected = in {labels:?}");
        assert!(
            labels.contains(&"IS NULL"),
            "expected IS NULL in {labels:?}"
        );
        assert!(labels.contains(&"LIKE"), "expected LIKE in {labels:?}");
        assert!(items.iter().all(|i| i.kind == CompletionKind::Operator));
    }

    #[test]
    fn complete_should_return_next_clause_candidates_for_next_clause_context() {
        let meta = DbMetadata::default();
        let items = CompletionEngine::complete(CompletionContext::NextClause, &meta, "");
        let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"WHERE"), "expected WHERE in {labels:?}");
        assert!(labels.contains(&"JOIN"), "expected JOIN in {labels:?}");
        assert!(
            labels.contains(&"ORDER BY"),
            "expected ORDER BY in {labels:?}"
        );
        assert!(labels.contains(&"LIMIT"), "expected LIMIT in {labels:?}");
        assert!(items.iter().all(|i| i.kind == CompletionKind::Keyword));
    }

    #[test]
    fn complete_should_return_empty_for_none_context() {
        let meta = make_metadata();
        let items = CompletionEngine::complete(CompletionContext::None, &meta, "");
        assert!(items.is_empty());
    }

    #[test]
    fn complete_should_include_column_type_in_detail() {
        let meta = make_metadata();
        let ctx = CompletionContext::ColumnName {
            table: Some("users".to_string()),
        };
        // Use partial prefix so the exact-match filter does not exclude the column.
        let items = CompletionEngine::complete(ctx, &meta, "i");
        let id_item = items
            .iter()
            .find(|i| i.label == "id")
            .expect("id column not found");
        assert_eq!(id_item.detail, Some("integer".to_string()));
    }

    #[test]
    fn complete_should_be_case_insensitive_for_prefix() {
        let meta = DbMetadata::default();
        let upper = CompletionEngine::complete(CompletionContext::Keyword, &meta, "SEL");
        let lower = CompletionEngine::complete(CompletionContext::Keyword, &meta, "sel");
        let upper_labels: Vec<_> = upper.iter().map(|i| &i.label).collect();
        let lower_labels: Vec<_> = lower.iter().map(|i| &i.label).collect();
        assert_eq!(upper_labels, lower_labels);
    }

    #[test]
    fn complete_should_assign_view_kind_to_view_candidates() {
        let meta = make_metadata();
        let items = CompletionEngine::complete(CompletionContext::TableName, &meta, "");
        let view_item = items
            .iter()
            .find(|i| i.label == "active_users")
            .expect("view not found");
        let table_item = items
            .iter()
            .find(|i| i.label == "users")
            .expect("table not found");
        assert_eq!(view_item.kind, CompletionKind::View);
        assert_eq!(table_item.kind, CompletionKind::Table);
    }

    #[test]
    fn complete_should_set_insert_text_equal_to_label() {
        let meta = make_metadata();
        let items = CompletionEngine::complete(CompletionContext::TableName, &meta, "");
        assert!(items.iter().all(|i| i.insert_text == i.label));
        let kw_items = CompletionEngine::complete(CompletionContext::Keyword, &meta, "sel");
        assert!(kw_items.iter().all(|i| i.insert_text == i.label));
    }

    #[test]
    fn complete_should_return_value_candidates_for_value_expected_context() {
        let meta = DbMetadata::default();
        let items = CompletionEngine::complete(CompletionContext::ValueExpected, &meta, "");
        let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"''"), "expected '' in {labels:?}");
        assert!(labels.contains(&"NULL"), "expected NULL in {labels:?}");
    }

    #[test]
    fn complete_should_set_cursor_offset_one_for_empty_string_literal() {
        let meta = DbMetadata::default();
        let items = CompletionEngine::complete(CompletionContext::ValueExpected, &meta, "");
        let quote_item = items
            .iter()
            .find(|i| i.label == "''")
            .expect("'' not found");
        assert_eq!(quote_item.cursor_offset, 1);
    }

    #[test]
    fn complete_should_disambiguate_columns_with_same_name_in_multiple_tables() {
        // Two tables each have a "name" column — labels should carry the table qualifier.
        let meta = DbMetadata {
            tables: vec![
                TableInfo {
                    name: "users".to_string(),
                    columns: vec![ColumnInfo {
                        name: "name".to_string(),
                        data_type: "varchar".to_string(),
                        nullable: false,
                    }],
                },
                TableInfo {
                    name: "companies".to_string(),
                    columns: vec![ColumnInfo {
                        name: "name".to_string(),
                        data_type: "varchar".to_string(),
                        nullable: false,
                    }],
                },
            ],
            views: vec![],
            stored_procs: vec![],
            indexes: vec![],
        };
        let ctx = CompletionContext::ColumnName { table: None };
        let items = CompletionEngine::complete(ctx, &meta, "na");
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(
            labels.contains(&"name (users)"),
            "expected 'name (users)' in {labels:?}"
        );
        assert!(
            labels.contains(&"name (companies)"),
            "expected 'name (companies)' in {labels:?}"
        );
        // insert_text should still be the plain column name
        assert!(items.iter().all(|i| i.insert_text == "name"));
        // table_name carries the owning table
        let users_item = items.iter().find(|i| i.label == "name (users)").unwrap();
        assert_eq!(users_item.table_name.as_deref(), Some("users"));
    }

    #[test]
    fn complete_should_narrow_disambiguated_column_by_typing_table_name_suffix() {
        // Two tables share a "name" column.  Typing "nameuse" should narrow to
        // only "name (users)" by matching the table-name suffix "use" → "users".
        let meta = DbMetadata {
            tables: vec![
                TableInfo {
                    name: "users".to_string(),
                    columns: vec![ColumnInfo {
                        name: "name".to_string(),
                        data_type: "varchar".to_string(),
                        nullable: false,
                    }],
                },
                TableInfo {
                    name: "companies".to_string(),
                    columns: vec![ColumnInfo {
                        name: "name".to_string(),
                        data_type: "varchar".to_string(),
                        nullable: false,
                    }],
                },
            ],
            views: vec![],
            stored_procs: vec![],
            indexes: vec![],
        };
        let ctx = CompletionContext::ColumnName { table: None };
        // "nameuse" → col part "name" matches both; table suffix "use" only matches "users"
        let items = CompletionEngine::complete(ctx.clone(), &meta, "nameuse");
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(
            labels,
            vec!["name (users)"],
            "expected only 'name (users)' in {labels:?}"
        );
        // insert_text is still just the column name, not the full typed prefix
        assert_eq!(items[0].insert_text, "name");
        // "namecomp" → matches "name (companies)"
        let items2 = CompletionEngine::complete(ctx, &meta, "namecomp");
        let labels2: Vec<&str> = items2.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(
            labels2,
            vec!["name (companies)"],
            "expected only 'name (companies)' in {labels2:?}"
        );
    }

    #[test]
    fn complete_should_not_disambiguate_unique_column_names_when_table_is_none() {
        let meta = make_metadata(); // users.id, users.email, orders.order_id, orders.total, ...
        let ctx = CompletionContext::ColumnName { table: None };
        let items = CompletionEngine::complete(ctx, &meta, "");
        // "id" only exists in users → no disambiguation
        let id_item = items.iter().find(|i| i.insert_text == "id").unwrap();
        assert_eq!(id_item.label, "id");
        assert_eq!(id_item.table_name.as_deref(), Some("users"));
    }

    #[test]
    fn complete_should_carry_table_name_on_column_candidates_with_explicit_table() {
        let meta = make_metadata();
        let ctx = CompletionContext::ColumnName {
            table: Some("users".to_string()),
        };
        let items = CompletionEngine::complete(ctx, &meta, "");
        assert!(
            items
                .iter()
                .all(|i| i.table_name.as_deref() == Some("users"))
        );
    }

    #[test]
    fn complete_should_return_join_condition_tables_filtered_by_prefix() {
        let meta = DbMetadata::default();
        let tables = vec!["users".to_string(), "departments".to_string()];
        let ctx = CompletionContext::JoinConditionTable { tables };
        let items = CompletionEngine::complete(ctx.clone(), &meta, "dep");
        let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, vec!["departments"]);
        let all_items = CompletionEngine::complete(ctx, &meta, "");
        assert_eq!(all_items.len(), 2);
    }
}
