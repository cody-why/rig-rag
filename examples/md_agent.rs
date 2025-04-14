use anyhow::{Context, Result};
use rig::{
    Embed, cli_chatbot::cli_chatbot, embeddings::EmbeddingsBuilder, loaders::FileLoader,
    providers::openai, vector_store::in_memory_store::InMemoryVectorStore,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::{info, warn};

#[derive(Embed, Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
struct Document {
    id: String,
    #[embed]
    content: String,
}

fn load_files(path: PathBuf, exclude_file: &str) -> Result<Vec<(String, Vec<String>)>> {
    const CHUNK_SIZE: usize = 2000;

    let content_chunks = FileLoader::with_glob(path.to_str().context("Invalid path")?)?
        .read_with_path()
        .into_iter()
        .filter_map(|result| result.ok())
        .filter(|(path, _)| !path.to_str().unwrap().contains(exclude_file))
        .map(|(path, content)| {
            let chunks = chunk_text(&content, CHUNK_SIZE);

            let filename =
                path.file_name().and_then(|name| name.to_str()).unwrap_or("unknown").to_string();

            (filename, chunks)
        })
        .collect::<Vec<(_, _)>>();
    Ok(content_chunks)
}

/// 智能分块文本，尝试在句子边界处分割
fn chunk_text(text: &str, chunk_size: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current_chunk = String::new();
    let mut current_size = 0;

    // 按段落分割文本
    for paragraph in text.split("\n\n") {
        // 如果段落本身超过块大小，需要进一步分割
        if paragraph.len() > chunk_size {
            // 按句子分割段落
            for sentence in paragraph.split(&['.', '。', '!', '?']) {
                let sentence = sentence.trim();
                if sentence.is_empty() {
                    continue;
                }

                let sentence_with_punct = format!("{}. ", sentence);

                // 如果当前块加上这个句子会超出大小限制
                if current_size + sentence_with_punct.len() > chunk_size && current_size > 0 {
                    chunks.push(current_chunk.trim().to_string());
                    current_chunk = String::new();
                    current_size = 0;
                }

                current_chunk.push_str(&sentence_with_punct);
                current_size += sentence_with_punct.len();
            }
        } else {
            // 段落可以作为一个整体添加
            let paragraph_with_newlines = format!("{}\n\n", paragraph);

            // 如果当前块加上这个段落会超出大小限制
            if current_size + paragraph_with_newlines.len() > chunk_size && current_size > 0 {
                chunks.push(current_chunk.trim().to_string());
                current_chunk = String::new();
                current_size = 0;
            }

            current_chunk.push_str(&paragraph_with_newlines);
            current_size += paragraph_with_newlines.len();
        }
    }

    // 添加最后一个块
    if !current_chunk.is_empty() {
        chunks.push(current_chunk.trim().to_string());
    }

    chunks
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    info!("Starting Agent");

    dotenv::dotenv().ok();

    let api_key = std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY not set");
    let base_url = std::env::var("OPENAI_BASE_URL").expect("OPENAI_BASE_URL not set");
    let model_name = std::env::var("OPENAI_MODEL").expect("OPENAI_MODEL not set");

    let client = openai::Client::from_url(&api_key, &base_url);

    // 加载文件
    let documents_dir = std::env::current_dir()?.join("documents/*.*");
    info!("Loading documents from {}", documents_dir.display());

    let md_chunks =
        load_files(documents_dir, "preamble.txt").context("Failed to load documents")?;

    info!(
        "Successfully loaded and chunked {} document chunks",
        md_chunks.iter().fold(0, |acc, (_, chunks)| acc + chunks.len())
    );

    // 创建嵌入模型
    // 英文使用nomic-embed-text, 中文使用bge-m3
    let ollama_client = openai::Client::from_url("Ollama", "http://localhost:11434/v1");
    let model = ollama_client.embedding_model("bge-m3");

    // 创建嵌入构建器
    let mut builder = EmbeddingsBuilder::new(model.clone());

    // 添加来自 markdown 文档的块
    for (i, (source, contents)) in md_chunks.into_iter().enumerate() {
        println!("{} {} chunks: {}", i + 1, source, contents.len());
        for content in contents {
            builder = builder.document(Document {
                id: format!("document{}", i + 1),
                content,
            })?;
        }
    }

    // 构建嵌入
    info!("Generating embeddings...");
    let embeddings = builder.build().await?;
    info!("Successfully generated embeddings");
    let len = embeddings.len();
    // 创建向量存储和索引
    let vector_store = InMemoryVectorStore::from_documents(embeddings);
    let index = vector_store.index(model);
    info!("Successfully created vector store and index");

    // 加载预设提示
    let preamble = include_str!("../documents/preamble.txt");

    // 创建 RAG 代理
    info!("Initializing RAG agent...");
    let rag_agent = client
        .agent(&model_name)
        .temperature(0.3) // 0.1-0.3 准确性高，0.5-0.7 创造性高
        .preamble(preamble)
        .dynamic_context(len, index)
        .build();

    info!("Starting CLI chatbot...");

    // 启动交互式 CLI
    if let Err(e) = cli_chatbot(rag_agent).await {
        warn!("Error in CLI chatbot: {}", e);
    }

    info!("Shutting down");
    Ok(())
}
