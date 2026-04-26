#[derive(Debug, Clone)]
pub struct CompletionItem {
    pub label: String,
    pub kind: CompletionKind,
    pub insert_text: String,
    /// Byte offset within `insert_text` where the cursor should land after insertion.
    /// `0` means "end of inserted text" (default).  A positive value places the cursor
    /// that many bytes from the start (e.g. `1` for `''` puts the cursor between quotes).
    pub cursor_offset: i32,
    pub detail: Option<String>,
    /// For column candidates, the owning table name. Used by the UI to auto-append
    /// `FROM <table>` when a column is accepted before any FROM clause is present.
    pub table_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CompletionKind {
    Table,
    Column,
    Keyword,
    Operator,
    Schema,
    View,
}

pub mod cache;
pub mod engine;
pub mod parser;
pub mod service;
