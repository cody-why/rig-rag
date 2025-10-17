mod conversation_store;
pub mod lancedb_store;
mod user_store;

pub use conversation_store::*;
pub use lancedb_store::*;
pub use user_store::*;

// alias for DocumentStore
pub type DocumentStore = lancedb_store::DocumentStore<rig::providers::openai::EmbeddingModel>;
