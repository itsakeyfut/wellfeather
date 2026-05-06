use crate::CompletionKind;
use crate::parser::CompletionContext;
use wf_db::models::{ColumnInfo, DbMetadata, TableInfo};

use super::CompletionEngine;

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
