use std::collections::HashMap;

use wf_db::models::DbMetadata;

use crate::{CompletionItem, CompletionKind};

// Value literal candidates shown after a comparison operator (=, <, >).
pub(super) const VALUE_CANDIDATES: &[(&str, i32)] = &[
    ("''", 1), // cursor_offset 1 → places cursor between quotes
    ("NULL", 0),
    ("TRUE", 0),
    ("FALSE", 0),
];

pub(super) const SQL_KEYWORDS: &[&str] = &[
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

pub(super) fn next_clause_candidates() -> Vec<CompletionItem> {
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

pub(super) fn join_on_candidates() -> Vec<CompletionItem> {
    vec![CompletionItem {
        label: "ON".to_string(),
        kind: CompletionKind::Keyword,
        insert_text: "ON".to_string(),
        cursor_offset: 0,
        detail: None,
        table_name: None,
    }]
}

pub(super) fn operator_candidates() -> Vec<CompletionItem> {
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

pub(super) fn value_candidates() -> Vec<CompletionItem> {
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

pub(super) fn keyword_candidates(prefix_upper: &str) -> Vec<CompletionItem> {
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

pub(super) fn table_candidates(metadata: &DbMetadata, prefix_upper: &str) -> Vec<CompletionItem> {
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

pub(super) fn column_candidates(
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
            let mut tables_per_col: HashMap<String, usize> = HashMap::new();
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
