#[derive(Debug, Clone)]
pub struct CompletionItem {
    pub label: String,
    pub kind: CompletionKind,
    pub insert_text: String,
    pub detail: Option<String>,
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
