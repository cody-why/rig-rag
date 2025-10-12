pub mod lancedb_store;
mod user_store;

pub use lancedb_store::*;
pub use user_store::*;

// 兼容现有用法：为 OpenAI 的 EmbeddingModel 提供具体化别名
pub type DocumentStore = lancedb_store::DocumentStore<rig::providers::openai::EmbeddingModel>;
pub type DocumentStoreRef = lancedb_store::DocumentStoreRef<rig::providers::openai::EmbeddingModel>;
