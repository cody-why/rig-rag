use anyhow::{Context, Result};
use rig::loaders::FileLoader;
use std::path::PathBuf;

pub struct FileChunk {
    pub filename: String,
    pub chunks: Vec<String>,
}

impl FileChunk {
    pub fn load_files(path: PathBuf, exclude_file: &str) -> Result<Vec<FileChunk>> {
        const CHUNK_SIZE: usize = 2000;

        let content_chunks = FileLoader::with_glob(path.to_str().context("Invalid path")?)?
            .read_with_path()
            .into_iter()
            .filter_map(|result| result.ok())
            .filter(|(path, _)| !path.to_str().unwrap().contains(exclude_file))
            .map(|(path, content)| {
                let chunks = chunk_text(&content, CHUNK_SIZE);

                let filename = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("unknown")
                    .to_string();

                FileChunk { filename, chunks }
            })
            .collect::<Vec<FileChunk>>();
        Ok(content_chunks)
    }
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
