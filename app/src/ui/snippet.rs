use std::rc::Rc;
use std::sync::Arc;

use slint::ComponentHandle;
use wf_config::snippet::SnippetRepository;

use super::with_ui;

pub(super) fn snippet_to_slint(b: wf_config::snippet::SnippetEntry) -> crate::SnippetEntry {
    crate::SnippetEntry {
        id: b.id.into(),
        name: b.name.into(),
        comment: b.comment.into(),
        sql: b.sql.into(),
    }
}

pub(super) async fn do_refresh_snippets(
    ww: &slint::Weak<crate::AppWindow>,
    repo: &SnippetRepository,
    connection_id: Option<&str>,
) {
    let items = repo.list(connection_id).await.unwrap_or_default();
    let slint_items: Vec<crate::SnippetEntry> = items.into_iter().map(snippet_to_slint).collect();
    let ww_c = ww.clone();
    let _ = slint::invoke_from_event_loop(move || {
        with_ui(&ww_c, move |ui| {
            ui.set_snippets(Rc::new(slint::VecModel::from(slint_items)).into());
        });
    });
}

pub(super) fn register_snippet_callbacks(
    window: &crate::AppWindow,
    snippet_repo: Arc<SnippetRepository>,
) {
    let ui = window.global::<crate::UiState>();

    // open-snippet-save: extract SQL from selection or cursor line, pre-fill dialog.
    {
        let ww = window.as_weak();
        ui.on_open_snippet_save(move |anchor_pos, cursor_pos| {
            let Some(w) = ww.upgrade() else { return };
            let ui = w.global::<crate::UiState>();
            let editor_text = ui.get_editor_text().to_string();
            let a = (anchor_pos as usize).min(editor_text.len());
            let c = (cursor_pos as usize).min(editor_text.len());
            let (start, end) = (a.min(c), a.max(c));
            let sql = if start < end {
                editor_text[start..end].to_string()
            } else {
                // Extract the cursor's line from the full editor text.
                let byte_pos = c;
                let line_start = editor_text[..byte_pos]
                    .rfind('\n')
                    .map(|i| i + 1)
                    .unwrap_or(0);
                let line_end = editor_text[byte_pos..]
                    .find('\n')
                    .map(|i| byte_pos + i)
                    .unwrap_or(editor_text.len());
                editor_text[line_start..line_end].trim().to_string()
            };
            if sql.is_empty() {
                return;
            }
            ui.set_snippet_save_sql(sql.into());
            ui.set_snippet_save_comment("".into());
            ui.set_show_snippet_save(true);
        });
    }

    // save-snippet: persist the new entry with a sequential "Query N" name, refresh.
    {
        let repo = Arc::clone(&snippet_repo);
        let ww = window.as_weak();
        ui.on_save_snippet(move |_name, comment, sql| {
            let Some(w) = ww.upgrade() else { return };
            let ui = w.global::<crate::UiState>();
            let conn_id_str = ui.get_active_connection_id().to_string();
            let conn_id = if conn_id_str.is_empty() {
                None
            } else {
                Some(conn_id_str)
            };
            let comment = comment.to_string();
            let sql = sql.to_string();
            let repo_c = Arc::clone(&repo);
            let ww_c = ww.clone();
            tokio::spawn(async move {
                let n = repo_c.next_query_number().await.unwrap_or(1);
                let name = format!("Query {n}");
                let entry = wf_config::snippet::SnippetEntry {
                    id: uuid::Uuid::new_v4().to_string(),
                    name,
                    comment,
                    connection_id: None,
                    sql,
                    created_at: chrono::Utc::now().to_rfc3339(),
                    sort_order: 0,
                };
                if let Err(e) = repo_c.add(&entry).await {
                    tracing::warn!(error = %e, "failed to save snippet");
                    return;
                }
                do_refresh_snippets(&ww_c, &repo_c, conn_id.as_deref()).await;
            });
        });
    }

    // save-snippet-edit: persist updated comment + sql, close edit dialog, refresh.
    {
        let repo = Arc::clone(&snippet_repo);
        let ww = window.as_weak();
        ui.on_save_snippet_edit(move |id, comment, sql| {
            let Some(w) = ww.upgrade() else { return };
            let ui = w.global::<crate::UiState>();
            let conn_id_str = ui.get_active_connection_id().to_string();
            let conn_id = if conn_id_str.is_empty() {
                None
            } else {
                Some(conn_id_str)
            };
            let id = id.to_string();
            let comment = comment.to_string();
            let sql = sql.to_string();
            let repo_c = Arc::clone(&repo);
            let ww_c = ww.clone();
            tokio::spawn(async move {
                if let Err(e) = repo_c.update(&id, &comment, &sql).await {
                    tracing::warn!(error = %e, "failed to update snippet");
                    return;
                }
                do_refresh_snippets(&ww_c, &repo_c, conn_id.as_deref()).await;
            });
        });
    }

    // delete-snippet-item: remove from DB, refresh list.
    {
        let repo = Arc::clone(&snippet_repo);
        let ww = window.as_weak();
        ui.on_delete_snippet_item(move |id| {
            let Some(w) = ww.upgrade() else { return };
            let ui = w.global::<crate::UiState>();
            let conn_id_str = ui.get_active_connection_id().to_string();
            let conn_id = if conn_id_str.is_empty() {
                None
            } else {
                Some(conn_id_str)
            };
            let id = id.to_string();
            let repo_c = Arc::clone(&repo);
            let ww_c = ww.clone();
            tokio::spawn(async move {
                if let Err(e) = repo_c.delete(&id).await {
                    tracing::warn!(error = %e, "failed to delete snippet");
                    return;
                }
                do_refresh_snippets(&ww_c, &repo_c, conn_id.as_deref()).await;
            });
        });
    }

    // load-snippet-sql: insert SQL at editor cursor position.
    {
        let ww = window.as_weak();
        ui.on_load_snippet_sql(move |sql, cursor_pos| {
            with_ui(&ww, |ui| {
                let current = ui.get_editor_text().to_string();
                let pos = (cursor_pos as usize).min(current.len());
                let new_text = format!("{}{}{}", &current[..pos], sql, &current[pos..]);
                let new_cursor = (pos + sql.len()) as i32;
                ui.set_editor_text(new_text.into());
                ui.set_editor_cursor_target(new_cursor);
            });
        });
    }

    // execute-snippet-sql: run snippet SQL directly without touching the editor.
    {
        let ww = window.as_weak();
        ui.on_execute_snippet_sql(move |sql| {
            let Some(w) = ww.upgrade() else { return };
            let ui = w.global::<crate::UiState>();
            ui.invoke_run_query(sql);
        });
    }

    // save-snippet-bar-position: persist dragged position.
    {
        let repo = Arc::clone(&snippet_repo);
        ui.on_save_snippet_bar_position(move |x, y| {
            let repo_c = Arc::clone(&repo);
            tokio::spawn(async move {
                if let Err(e) = repo_c.set_bar_position(x, y).await {
                    tracing::warn!(error = %e, "failed to save snippet bar position");
                }
            });
        });
    }
}
