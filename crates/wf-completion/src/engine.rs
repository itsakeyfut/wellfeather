//! Completion candidate generation.
//!
//! [`CompletionEngine::complete`] maps a [`CompletionContext`] + [`DbMetadata`] + prefix
//! string to a filtered list of [`CompletionItem`] candidates.

mod candidates;
#[cfg(test)]
mod tests;

use wf_db::models::DbMetadata;

use crate::parser::CompletionContext;
use crate::{CompletionItem, CompletionKind};

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
                candidates::keyword_candidates(&prefix_upper)
                    .into_iter()
                    .filter(|item| item.label.to_ascii_uppercase() != prefix_upper)
                    .collect()
            }
            CompletionContext::TableName => {
                let tables: Vec<_> = candidates::table_candidates(metadata, &prefix_upper)
                    .into_iter()
                    .filter(|item| item.label.to_ascii_uppercase() != prefix_upper)
                    .collect();
                if tables.is_empty() && !prefix_upper.is_empty() {
                    // No table matched the prefix — the user is likely typing a
                    // keyword (e.g. "WHERE" after "FROM users wh").  Fall back to
                    // keyword suggestions so structure keywords always surface.
                    candidates::keyword_candidates(&prefix_upper)
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
                let cols: Vec<_> =
                    candidates::column_candidates(metadata, table.as_deref(), &prefix_upper)
                        .into_iter()
                        .filter(|item| item.label.to_ascii_uppercase() != prefix_upper)
                        .collect();
                if cols.is_empty() && !prefix_upper.is_empty() {
                    // No column matched the prefix — fall back to keywords so the
                    // user can continue structuring the query (AND, OR, ORDER BY …).
                    candidates::keyword_candidates(&prefix_upper)
                        .into_iter()
                        .filter(|item| item.label.to_ascii_uppercase() != prefix_upper)
                        .collect()
                } else {
                    cols
                }
            }
            CompletionContext::NextClause => candidates::next_clause_candidates(),
            CompletionContext::JoinOn => candidates::join_on_candidates(),
            CompletionContext::Operator => candidates::operator_candidates(),
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
            CompletionContext::ValueExpected => candidates::value_candidates(),
            CompletionContext::None => vec![],
        }
    }
}
