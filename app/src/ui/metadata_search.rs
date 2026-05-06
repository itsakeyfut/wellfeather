use std::rc::Rc;
use std::sync::{Arc, Mutex};

use rust_i18n::t;
use slint::ComponentHandle;
use wf_db::models::DbMetadata;

use super::{SidebarUiState, with_sidebar};

/// A single metadata search result (not Slint-specific).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SearchResultItem {
    pub(crate) kind: &'static str, // "table" | "view" | "column"
    pub(crate) label: String,
    pub(crate) detail: String,
    pub(crate) table_name: String,
}

/// Search `meta` for `query` (case-insensitive).
///
/// Empty query returns all tables and views (no columns).
/// Non-empty query searches table names, view names, and column names.
/// Prefix matches are ranked before substring matches. Results are capped at 50.
pub(crate) fn search_metadata(query: &str, meta: &DbMetadata) -> Vec<SearchResultItem> {
    if query.is_empty() {
        let mut results: Vec<SearchResultItem> = meta
            .tables
            .iter()
            .map(|t| SearchResultItem {
                kind: "table",
                label: t.name.clone(),
                detail: String::new(),
                table_name: t.name.clone(),
            })
            .chain(meta.views.iter().map(|v| SearchResultItem {
                kind: "view",
                label: v.name.clone(),
                detail: String::new(),
                table_name: v.name.clone(),
            }))
            .collect();
        results.truncate(50);
        return results;
    }

    let q = query.to_lowercase();
    let mut prefix: Vec<SearchResultItem> = vec![];
    let mut contains: Vec<SearchResultItem> = vec![];

    let classify = |item: SearchResultItem,
                    name_lower: &str,
                    p: &mut Vec<SearchResultItem>,
                    c: &mut Vec<SearchResultItem>| {
        if name_lower.starts_with(q.as_str()) {
            p.push(item);
        } else if name_lower.contains(q.as_str()) {
            c.push(item);
        }
    };

    for table in &meta.tables {
        let name_lower = table.name.to_lowercase();
        classify(
            SearchResultItem {
                kind: "table",
                label: table.name.clone(),
                detail: String::new(),
                table_name: table.name.clone(),
            },
            &name_lower,
            &mut prefix,
            &mut contains,
        );
        for col in &table.columns {
            let col_label = format!("{}.{}", table.name, col.name);
            let col_label_lower = col_label.to_lowercase();
            classify(
                SearchResultItem {
                    kind: "column",
                    label: col_label,
                    detail: col.data_type.clone(),
                    table_name: table.name.clone(),
                },
                &col_label_lower,
                &mut prefix,
                &mut contains,
            );
        }
    }

    for view in &meta.views {
        let name_lower = view.name.to_lowercase();
        classify(
            SearchResultItem {
                kind: "view",
                label: view.name.clone(),
                detail: String::new(),
                table_name: view.name.clone(),
            },
            &name_lower,
            &mut prefix,
            &mut contains,
        );
    }

    prefix.extend(contains);
    prefix.truncate(50);
    prefix
}

/// Convert `SearchResultItem`s to the Slint `MetadataSearchResult` type.
fn items_to_slint(items: Vec<SearchResultItem>) -> Vec<crate::MetadataSearchResult> {
    items
        .into_iter()
        .map(|r| crate::MetadataSearchResult {
            kind: r.kind.into(),
            label: r.label.into(),
            detail: r.detail.into(),
            table_name: r.table_name.into(),
        })
        .collect()
}

pub(super) fn register_metadata_search_callbacks(
    window: &crate::AppWindow,
    sidebar_state: Arc<Mutex<SidebarUiState>>,
) {
    let ui = window.global::<crate::UiState>();

    // metadata-search-open: check active connection; open palette if connected.
    {
        let window_weak = window.as_weak(); // clone required: on_metadata_search_open closure
        let sidebar_state_open = Arc::clone(&sidebar_state); // clone required: captured by open closure
        ui.on_metadata_search_open(move || {
            let Some(w) = window_weak.upgrade() else {
                return;
            };
            let ui = w.global::<crate::UiState>();
            if ui.get_active_connection_id().is_empty() {
                ui.set_status_message(t!("error.no_active_connection").as_ref().into());
                return;
            }
            ui.set_metadata_search_query("".into());
            ui.set_metadata_search_selected(0);
            // Show all tables/views immediately on open (empty query).
            let active_id = ui.get_active_connection_id().to_string();
            let sb = sidebar_state_open.lock().unwrap_or_else(|p| p.into_inner());
            let items = if let Some(meta) = sb.metadata.get(&active_id) {
                search_metadata("", meta)
            } else {
                vec![]
            };
            drop(sb);
            let slint_items = items_to_slint(items);
            ui.set_metadata_search_results(Rc::new(slint::VecModel::from(slint_items)).into());
            ui.set_show_metadata_search(true);
        });
    }

    // metadata-search: recompute results for the current query.
    {
        let window_weak = window.as_weak(); // clone required: on_metadata_search closure
        let sidebar_state = Arc::clone(&sidebar_state); // clone required: on_metadata_search closure
        ui.on_metadata_search(move || {
            let Some(w) = window_weak.upgrade() else {
                return;
            };
            let ui = w.global::<crate::UiState>();
            let query = ui.get_metadata_search_query().to_string();
            let active_id = ui.get_active_connection_id().to_string();
            let items = with_sidebar(&sidebar_state, |sb| {
                if let Some(meta) = sb.metadata.get(&active_id) {
                    search_metadata(&query, meta)
                } else {
                    vec![]
                }
            });
            let slint_items = items_to_slint(items);
            ui.set_metadata_search_results(Rc::new(slint::VecModel::from(slint_items)).into());
            ui.set_metadata_search_selected(0);
        });
    }

    // metadata-search-select: dispatch action based on result kind.
    {
        let window_weak = window.as_weak(); // clone required: on_metadata_search_select closure
        ui.on_metadata_search_select(move |kind, label, table_name| {
            let Some(w) = window_weak.upgrade() else {
                return;
            };
            let ui = w.global::<crate::UiState>();
            ui.set_show_metadata_search(false);
            ui.set_metadata_search_query("".into());
            match kind.as_str() {
                "table" | "view" => {
                    ui.invoke_table_double_clicked(label);
                }
                "column" => {
                    // Copy just the column name (not "table.column") to clipboard.
                    let col_name = label
                        .as_str()
                        .split('.')
                        .next_back()
                        .unwrap_or(label.as_str())
                        .to_string();
                    if let Ok(mut clip) = arboard::Clipboard::new() {
                        let _ = clip.set_text(col_name);
                    }
                    // Open the parent table.
                    ui.invoke_table_double_clicked(table_name);
                }
                _ => {}
            }
        });
    }
}
