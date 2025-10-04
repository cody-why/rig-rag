pub mod lancedb_store;

pub use lancedb_store::*;

// 兼容现有用法：为 OpenAI 的 EmbeddingModel 提供具体化别名
pub type DocumentStore = lancedb_store::DocumentStore<rig::providers::openai::EmbeddingModel>;
pub type DocumentStoreWrapper =
    lancedb_store::DocumentStoreWrapper<rig::providers::openai::EmbeddingModel>;
