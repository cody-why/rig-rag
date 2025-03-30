use std::sync::{Arc, RwLock};

use rig::{
    Embed, agent::Agent, embeddings::EmbeddingsBuilder, providers::openai,
    vector_store::in_memory_store::InMemoryVectorStore,
};
use serde::{Deserialize, Serialize};

use super::{RigAgentBuilder, file_chunk::FileChunk};

#[derive(Clone)]
pub struct RigAgent {
    pub agent: Arc<Agent<openai::CompletionModel>>,
    pub context: Arc<RwLock<RigAgentContext>>,
}

#[derive(Clone)]
pub struct RigAgentContext {
    pub preamble: String,
    pub temperature: f64,
    pub openai_model: String,
    pub client: openai::Client,
    pub embedding_model: openai::EmbeddingModel,
    pub vector_store: InMemoryVectorStore<Document>,
}

#[derive(Embed, Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct Document {
    pub id: String,
    #[embed]
    pub content: String,
}

impl RigAgent {
    pub fn builder() -> RigAgentBuilder {
        RigAgentBuilder::default()
    }

    pub fn from_env() -> RigAgentBuilder {
        RigAgentBuilder::from_env()
    }

    pub async fn add_documents(&mut self, documents: Vec<FileChunk>) -> anyhow::Result<()> {
        let embedding_model = self.context.read().unwrap().embedding_model.clone();
        let mut vector_store = self.context.read().unwrap().vector_store.clone();

        // 创建嵌入构建器
        let mut builder = EmbeddingsBuilder::new(embedding_model);

        // 添加来自 markdown 文档的块
        for (i, doc) in documents.into_iter().enumerate() {
            println!("{} {} chunks: {}", i + 1, doc.filename, doc.chunks.len());
            for content in doc.chunks {
                builder = builder.document(Document {
                    id: format!("document{}", i + 1),
                    content,
                })?;
            }
        }

        // 构建嵌入
        let embeddings = builder.build().await?;
        vector_store.add_documents(embeddings);

        self.context.write().unwrap().vector_store = vector_store;

        Ok(())
    }
}

impl RigAgentContext {
    pub fn build(&self) -> Agent<openai::CompletionModel> {
        let index = self.vector_store.clone().index(self.embedding_model.clone());
        let len = index.len();
        self.client
            .agent(&self.openai_model)
            .temperature(self.temperature) // 0.1-0.3 准确性高，0.5-0.7 创造性高
            .preamble(&self.preamble)
            .dynamic_context(len, index)
            .build()
    }
}
