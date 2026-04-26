//! Completion candidate generation.
//!
//! [`CompletionEngine::complete`] maps a [`CompletionContext`] + [`DbMetadata`] + prefix
//! string to a filtered list of [`CompletionItem`] candidates.

use wf_db::models::{DbMetadata, TableInfo};

use crate::parser::CompletionContext;
use crate::{CompletionItem, CompletionKind};

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
                let tables = table_candidates(metadata, &prefix_upper);
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
                let cols = column_candidates(metadata, table.as_deref(), &prefix_upper);
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
            detail: None,
        })
        .collect()
}

fn join_on_candidates() -> Vec<CompletionItem> {
    vec![CompletionItem {
        label: "ON".to_string(),
        kind: CompletionKind::Keyword,
        insert_text: "ON".to_string(),
        detail: None,
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
            detail: None,
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
            detail: None,
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
            detail: None,
        })
        .collect()
}

fn column_candidates(
    metadata: &DbMetadata,
    table: Option<&str>,
    prefix_upper: &str,
) -> Vec<CompletionItem> {
    let sources: Vec<&TableInfo> = match table {
        Some(name) => {
            let name_upper = name.to_ascii_uppercase();
            metadata
                .tables
                .iter()
                .chain(metadata.views.iter())
                .filter(|t| t.name.to_ascii_uppercase() == name_upper)
                .collect()
        }
        None => metadata
            .tables
            .iter()
            .chain(metadata.views.iter())
            .collect(),
    };

    sources
        .into_iter()
        .flat_map(|t| t.columns.iter())
        .filter(|c| c.name.to_ascii_uppercase().starts_with(prefix_upper))
        .map(|c| CompletionItem {
            label: c.name.clone(),
            kind: CompletionKind::Column,
            insert_text: c.name.clone(),
            detail: Some(c.data_type.clone()),
        })
        .collect()
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
        let items = CompletionEngine::complete(ctx, &meta, "id");
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
}
