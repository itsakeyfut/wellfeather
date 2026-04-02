#[derive(Debug, Clone)]
pub struct CompletionItem {
    pub label: String,
    pub kind: CompletionKind,
    pub detail: Option<String>,
}

#[derive(Debug, Clone)]
pub enum CompletionKind {
    Table,
    Column,
    Keyword,
    Schema,
}

pub mod cache;
pub mod engine;
pub mod parser;
pub mod service;
