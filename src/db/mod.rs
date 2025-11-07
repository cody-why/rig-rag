mod conversation_store;
pub mod qdrant_store;
mod user_store;

pub use conversation_store::*;
pub use qdrant_store::*;
pub use user_store::*;

// alias for DocumentStore
pub type DocumentStore = qdrant_store::DocumentStore<rig::providers::openai::EmbeddingModel>;
