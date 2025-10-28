use std::{env, sync::Arc};

use anyhow::{Context, Result};
use arrow_array::{ArrayRef, FixedSizeListArray, RecordBatch, RecordBatchIterator, StringArray, types::Float64Type};
use lancedb::arrow::arrow_schema::{DataType, Field, Fields, Schema};
use rig::{Embed, OneOrMany, agent::Agent, completion::{Chat, Message}, embeddings::{Embedding, EmbeddingModel, EmbeddingsBuilder}, prelude::{CompletionClient, EmbeddingsClient}, providers::openai};
use rig_lancedb::{LanceDbVectorIndex, SearchParams};
use serde::Serialize;
use tracing::info;

/// Simplified reproduction code: Demonstrates base64 message conversion error in rig-core 0.22
///
/// Chat failed: CompletionError: RequestError: Message conversion error: Documents must be base64
#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();
    tracing_subscriber::fmt::init();

    let openai_api_key = env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set");
    let embedding_api_key = env::var("EMBEDDING_API_KEY").expect("EMBEDDING_API_KEY must be set");

    println!("🚀 Starting reproduction code...");

    let client = openai::Client::builder(&openai_api_key)
        .base_url("https://api.siliconflow.cn/v1")
        .build()?;

    let embedding_client = openai::Client::builder(&embedding_api_key)
        .base_url("https://api.siliconflow.cn/v1")
        .build()?;

    let embedding_model = embedding_client.embedding_model("BAAI/bge-m3");

    // Try to build RAG agent with vector index
    println!("📚 Trying to build RAG agent with vector index...");

    match build_rag_agent(&client, &embedding_model).await {
        Ok(agent) => {
            println!("✅ RAG agent built successfully");

            // Try to chat
            println!("💬 Starting chat test...");

            let message = "Hello, please introduce yourself";
            let history = vec![
                Message::user("Previous message"),
                Message::assistant("Previous response"),
            ];

            match agent.chat(message, history).await {
                Ok(response) => {
                    println!("✅ Chat successful: {}", response);
                },
                Err(e) => {
                    println!("❌ Chat failed: {}", e);
                },
            }
        },
        Err(e) => {
            println!("❌ RAG agent build failed: {}", e);
        },
    }

    Ok(())
}

/// Build RAG agent with vector index
async fn build_rag_agent(
    client: &openai::Client, embedding_model: &openai::EmbeddingModel,
) -> Result<Agent<openai::CompletionModel>> {
    let db = init_lancedb(embedding_model).await?;
    let table_name = "test_table";
    let table = db.open_table(table_name).execute().await?;

    // Create vector index
    let search_params = SearchParams::default();
    let index =
        LanceDbVectorIndex::new(table, embedding_model.clone(), "id", search_params).await?;

    // Build RAG agent
    let top_k = 1;
    let agent = client
        // .agent("THUDM/GLM-4-9B-0414")
        .completion_model("THUDM/GLM-4-9B-0414")
        .completions_api()
        .into_agent_builder()
        .temperature(0.5)
        .preamble("You are a helpful AI assistant.")
        .dynamic_context(top_k, index)
        .build();

    Ok(agent)
}

/// Initialize LanceDB
async fn init_lancedb(embedding_model: &openai::EmbeddingModel) -> Result<lancedb::Connection> {
    let lancedb_path = env::var("LANCEDB_PATH").unwrap_or_else(|_| "data/lancedb".to_string());
    let table_name = "test_table";

    let db = lancedb::connect(&lancedb_path).execute().await?;
    // Connect to LanceDB
    let names = db.table_names().execute().await?;

    if names.contains(&table_name.to_string()) {
        println!(
            "✅ LanceDB table '{}' exists - ready for testing",
            table_name
        );
        return Ok(db);
    }
    let documents = vec![Document {
        id: "1".to_string(),
        content: "Hello, world!".to_string(),
        source: "sample".to_string(),
    }];
    add_documents(&db, table_name, documents, embedding_model).await?;

    Ok(db)
}

/// Document structure
#[derive(Debug, Clone, Serialize, Embed, PartialEq)]
pub struct Document {
    pub id: String,
    #[embed]
    pub content: String,
    pub source: String,
}

async fn add_documents(
    db: &lancedb::Connection, table_name: &str, documents: Vec<Document>,
    embedding_model: &openai::EmbeddingModel,
) -> Result<()> {
    if documents.is_empty() {
        return Ok(());
    }
    let len = documents.len();
    // build embeddings
    let embeddings = EmbeddingsBuilder::new(embedding_model.clone())
        .documents(documents)
        .context("Failed to create embeddings builder")?
        .build()
        .await
        .context("Failed to build embeddings")?;

    let dims = if let Some((_, emb)) = embeddings.first() {
        emb.first().vec.len()
    } else {
        embedding_model.ndims()
    };

    // the record batch
    let record_batch =
        as_record_batch(embeddings, dims).context("Failed to create record batch")?;
    let schema = create_schema(dims);

    db.create_table(
        table_name,
        RecordBatchIterator::new(vec![Ok(record_batch)], Arc::new(schema)),
    )
    .execute()
    .await
    .context("Failed to create new table")?;

    info!("Successfully added {} documents", len);
    Ok(())
}

fn as_record_batch(
    records: Vec<(Document, OneOrMany<Embedding>)>, dims: usize,
) -> Result<RecordBatch> {
    if records.is_empty() {
        return Err(anyhow::anyhow!(
            "Cannot create RecordBatch from empty records"
        ));
    }

    let ids = StringArray::from_iter_values(records.iter().map(|(doc, _)| doc.id.clone()));
    let contents =
        StringArray::from_iter_values(records.iter().map(|(doc, _)| doc.content.clone()));
    let sources = StringArray::from_iter_values(records.iter().map(|(doc, _)| doc.source.clone()));

    let embeddings = FixedSizeListArray::from_iter_primitive::<Float64Type, _, _>(
        records
            .into_iter()
            .map(|(_, embeddings)| {
                Some(
                    embeddings
                        .first()
                        .vec
                        .into_iter()
                        .map(Some)
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>(),
        dims as i32,
    );

    RecordBatch::try_from_iter(vec![
        ("id", Arc::new(ids) as ArrayRef),
        ("content", Arc::new(contents) as ArrayRef),
        ("source", Arc::new(sources) as ArrayRef),
        ("embedding", Arc::new(embeddings) as ArrayRef),
    ])
    .map_err(|e| anyhow::anyhow!("Failed to create RecordBatch: {}", e))
}

fn create_schema(dims: usize) -> Schema {
    Schema::new(Fields::from(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("content", DataType::Utf8, false),
        Field::new("source", DataType::Utf8, false),
        Field::new(
            "embedding",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float64, true)),
                dims as i32,
            ),
            false,
        ),
    ]))
}
