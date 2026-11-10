use std::{
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result};
use chrono::Utc;
use tracing::{debug, info, warn};

use crate::{
    agent::RigAgent,
    db::{Document, DocumentStore, EmbeddingHandle},
};

const SOURCE_PREFIX: &str = "dir/";
const MAX_CHUNK_SIZE: usize = 8192;
const DEFAULT_HASH_PATH: &str = "data/dir_docs_hashes.json";

/// 已索引文件的内容指纹（进程内 + 落盘），用于跳过未改动的文件
static INDEXED_CONTENT_HASHES: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);

fn hashes_file_path() -> PathBuf {
    PathBuf::from(
        std::env::var("DIR_DOCS_HASH_PATH").unwrap_or_else(|_| DEFAULT_HASH_PATH.to_string()),
    )
}

/// 从磁盘加载指纹；值为 hex，避免 u64 JSON 精度问题
fn load_persisted_hashes() -> HashMap<String, u64> {
    let path = hashes_file_path();
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return HashMap::new();
    };
    let Ok(map) = serde_json::from_str::<HashMap<String, String>>(&raw) else {
        warn!(path = %path.display(), "Invalid dir docs hash file, ignore");
        return HashMap::new();
    };
    map.into_iter()
        .filter_map(|(k, hex)| u64::from_str_radix(&hex, 16).ok().map(|h| (k, h)))
        .collect()
}

fn persist_hashes(hashes: &HashMap<String, u64>) {
    let path = hashes_file_path();
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        warn!(error = %e, "Failed to create dir for hash file");
        return;
    }
    let serializable: HashMap<String, String> = hashes
        .iter()
        .map(|(k, v)| (k.clone(), format!("{v:016x}")))
        .collect();
    match serde_json::to_string_pretty(&serializable) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                warn!(path = %path.display(), error = %e, "Failed to persist dir docs hashes");
            }
        }
        Err(e) => warn!(error = %e, "Failed to serialize dir docs hashes"),
    }
}

#[derive(Debug, Clone, Default)]
pub struct DirSyncResult {
    pub files: usize,
    pub chunks: usize,
    /// 是否有新增/修改/删除
    pub changed: bool,
}

/// 目录快照指纹：路径 + 大小 + mtime，用于轮询检测变更
fn directory_snapshot(root: &Path, exclude_dirs: &[String]) -> Result<u64> {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut files = collect_md_files(root, exclude_dirs).unwrap_or_default();
    files.sort();
    files.len().hash(&mut hasher);
    for path in files {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path.as_path())
            .to_string_lossy()
            .replace('\\', "/");
        rel.hash(&mut hasher);
        if let Ok(meta) = std::fs::metadata(&path) {
            meta.len().hash(&mut hasher);
            if let Ok(modified) = meta.modified() {
                modified.hash(&mut hasher);
            }
        }
    }
    Ok(hasher.finish())
}

fn content_hash(content: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    content.hash(&mut hasher);
    hasher.finish()
}

fn indexed_hashes() -> std::sync::MutexGuard<'static, Option<HashMap<String, u64>>> {
    INDEXED_CONTENT_HASHES.lock().unwrap_or_else(|e| e.into_inner())
}

/// 将配置目录下的 `.md` 同步进向量库（可检索）
///
/// - 以相对路径生成稳定 `base_id`
/// - 内容未变则跳过重嵌
/// - 清理已不在目录中的旧 `dir/` 文档
pub async fn sync_directory_documents(
    documents_dir: &str,
    exclude_dirs: &[String],
    document_store: &DocumentStore,
    embedding_model: EmbeddingHandle,
) -> Result<DirSyncResult> {
    let root = PathBuf::from(documents_dir);
    if !root.exists() {
        warn!(
            dir = %documents_dir,
            "DOCUMENTS_DIR does not exist, skip directory document sync"
        );
        return Ok(DirSyncResult::default());
    }
    if !root.is_dir() {
        warn!(
            dir = %documents_dir,
            "DOCUMENTS_DIR is not a directory, skip sync"
        );
        return Ok(DirSyncResult::default());
    }

    let md_files = collect_md_files(&root, exclude_dirs)?;
    info!(
        dir = %documents_dir,
        files = md_files.len(),
        exclude = ?exclude_dirs,
        "Syncing markdown documents from directory"
    );

    let mut current_base_ids = HashSet::new();
    let mut total_chunks = 0usize;
    let mut changed = false;
    let mut new_hashes = HashMap::new();
    // 优先用进程内缓存；冷启动则读落盘指纹，避免重启全量重嵌
    let prev_hashes = {
        let mut guard = indexed_hashes();
        if guard.is_none() {
            *guard = Some(load_persisted_hashes());
        }
        guard.clone().unwrap_or_default()
    };

    for abs_path in &md_files {
        let rel = abs_path
            .strip_prefix(&root)
            .unwrap_or(abs_path.as_path())
            .to_string_lossy()
            .replace('\\', "/");

        let base_id = stable_base_id(&rel);
        current_base_ids.insert(base_id.clone());

        let content = match tokio::fs::read_to_string(abs_path).await {
            Ok(c) => c,
            Err(e) => {
                warn!(file = %rel, error = %e, "Failed to read markdown file, skip");
                continue;
            }
        };

        if content.trim().is_empty() {
            warn!(file = %rel, "Empty markdown file, skip");
            continue;
        }

        let hash = content_hash(&content);
        new_hashes.insert(base_id.clone(), hash);

        if prev_hashes.get(&base_id) == Some(&hash) {
            debug!(file = %rel, "Unchanged markdown, skip re-index");
            continue;
        }

        changed = true;

        // 先删旧切片再写入，避免内容变更后残留
        let _ = document_store
            .delete_document(&format!("{}_CHUNKED", base_id))
            .await;
        let _ = document_store.delete_document(&base_id).await;

        let chunks = chunk_text(&content, MAX_CHUNK_SIZE);
        if chunks.is_empty() {
            continue;
        }

        let source = format!("{}{}", SOURCE_PREFIX, rel);
        let timestamp = Utc::now();
        let total = chunks.len();
        let documents: Vec<Document> = chunks
            .into_iter()
            .enumerate()
            .map(|(idx, chunk_content)| {
                let id = if total == 1 {
                    base_id.clone()
                } else {
                    format!("{}-{}", base_id, idx)
                };
                let source = if total > 1 {
                    format!("{} (Part {}/{})", source, idx + 1, total)
                } else {
                    source.clone()
                };
                Document {
                    id,
                    base_id: base_id.clone(),
                    chunk_index: if total == 1 {
                        None
                    } else {
                        Some(idx as u32)
                    },
                    content: chunk_content,
                    source,
                    created_at: timestamp,
                    updated_at: timestamp,
                }
            })
            .collect();

        total_chunks += documents.len();
        document_store
            .add_documents_with_embeddings(documents, embedding_model.clone())
            .await
            .with_context(|| format!("Failed to index directory document: {}", rel))?;

        info!(file = %rel, base_id = %base_id, "Indexed directory markdown");
    }

    // 清理目录中已删除文件对应的旧索引
    let removed = cleanup_stale_dir_docs(document_store, &current_base_ids).await?;
    if removed > 0 {
        changed = true;
        info!(removed, "Removed stale directory documents from index");
    }

    *indexed_hashes() = Some(new_hashes.clone());
    persist_hashes(&new_hashes);

    info!(
        files = md_files.len(),
        chunks = total_chunks,
        changed,
        "Directory document sync complete"
    );
    Ok(DirSyncResult {
        files: md_files.len(),
        chunks: total_chunks,
        changed,
    })
}

/// 后台轮询 DOCUMENTS_DIR：文件增删改后自动同步并标记 agent 重建
pub fn spawn_documents_dir_watcher(
    documents_dir: String,
    exclude_dirs: Vec<String>,
    interval_secs: u64,
    document_store: Arc<DocumentStore>,
    agent: Arc<RigAgent>,
) {
    let interval = Duration::from_secs(interval_secs.max(5));
    tokio::spawn(async move {
        let root = PathBuf::from(&documents_dir);
        let mut last_snapshot = directory_snapshot(&root, &exclude_dirs).ok();

        info!(
            dir = %documents_dir,
            exclude = ?exclude_dirs,
            interval_secs = interval.as_secs(),
            "Watching DOCUMENTS_DIR for changes"
        );

        loop {
            tokio::time::sleep(interval).await;

            let snapshot = match directory_snapshot(&root, &exclude_dirs) {
                Ok(s) => s,
                Err(e) => {
                    debug!("Failed to snapshot DOCUMENTS_DIR: {}", e);
                    continue;
                }
            };

            if last_snapshot == Some(snapshot) {
                continue;
            }

            info!(dir = %documents_dir, "DOCUMENTS_DIR changed, re-syncing...");
            let embedding_model = agent.context.read().embedding_model.clone();
            match sync_directory_documents(
                &documents_dir,
                &exclude_dirs,
                document_store.as_ref(),
                embedding_model,
            )
            .await
            {
                Ok(result) => {
                    last_snapshot = Some(snapshot);
                    if result.changed {
                        agent.set_needs_rebuild(true).await;
                        info!(
                            files = result.files,
                            chunks = result.chunks,
                            "DOCUMENTS_DIR sync applied after change"
                        );
                    } else {
                        debug!("DOCUMENTS_DIR snapshot changed but no content delta");
                    }
                }
                Err(e) => warn!("Failed to sync DOCUMENTS_DIR after change: {}", e),
            }
        }
    });
}

async fn cleanup_stale_dir_docs(
    document_store: &DocumentStore,
    current_base_ids: &HashSet<String>,
) -> Result<usize> {
    let mut stale_base_ids = HashSet::new();
    let mut offset = 0usize;
    loop {
        let (docs, total) = document_store
            .list_documents_paginated(200, offset, None)
            .await?;
        if docs.is_empty() {
            break;
        }
        for doc in docs {
            let is_dir_doc = doc.source.starts_with(SOURCE_PREFIX)
                || (doc.base_id.starts_with("dir")
                    && doc.base_id.len() == 19
                    && doc.base_id.chars().skip(3).all(|c| c.is_ascii_hexdigit()));
            if is_dir_doc && !current_base_ids.contains(&doc.base_id) {
                stale_base_ids.insert(doc.base_id);
            }
        }
        offset += 200;
        if offset >= total {
            break;
        }
    }

    let mut removed = 0usize;
    for base_id in stale_base_ids {
        document_store
            .delete_document(&format!("{}_CHUNKED", base_id))
            .await?;
        removed += 1;
    }
    Ok(removed)
}

fn stable_base_id(rel_path: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    rel_path.hash(&mut hasher);
    format!("dir{:016x}", hasher.finish())
}

fn collect_md_files(root: &Path, exclude_dirs: &[String]) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_md_files_rec(root, root, exclude_dirs, &mut files)?;
    files.sort();
    Ok(files)
}

/// 判断相对路径（目录）是否应被排除
fn is_excluded_dir(rel: &str, exclude_dirs: &[String]) -> bool {
    if rel.is_empty() || exclude_dirs.is_empty() {
        return false;
    }
    let rel = rel.trim_matches('/');
    for pat in exclude_dirs {
        if pat.contains('/') {
            if rel == pat.as_str() || rel.starts_with(&format!("{pat}/")) {
                return true;
            }
        } else if rel == pat.as_str() || rel.split('/').any(|seg| seg == pat) {
            return true;
        }
    }
    false
}

fn collect_md_files_rec(
    root: &Path,
    dir: &Path,
    exclude_dirs: &[String],
    out: &mut Vec<PathBuf>,
) -> Result<()> {
    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("Failed to read documents directory: {}", dir.display()))?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(path.as_path())
                .to_string_lossy()
                .replace('\\', "/");
            if is_excluded_dir(&rel, exclude_dirs) {
                // debug!(dir = %rel, "Skip excluded documents subdirectory");
                continue;
            }
            collect_md_files_rec(root, &path, exclude_dirs, out)?;
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("md"))
        {
            out.push(path);
        }
    }
    Ok(())
}

fn chunk_text(text: &str, chunk_size: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();

    for paragraph in text.split("\n\n") {
        if paragraph.is_empty() {
            continue;
        }
        let piece = format!("{}\n\n", paragraph);

        if piece.len() > chunk_size {
            if !current.trim().is_empty() {
                chunks.push(current.trim().to_string());
                current.clear();
            }
            for sentence in paragraph.split(['.', '。', '!', '?', '！', '？']) {
                let sentence = sentence.trim();
                if sentence.is_empty() {
                    continue;
                }
                let with_punct = format!("{}. ", sentence);
                if current.len() + with_punct.len() > chunk_size && !current.is_empty() {
                    chunks.push(current.trim().to_string());
                    current.clear();
                }
                current.push_str(&with_punct);
            }
            continue;
        }

        if current.len() + piece.len() > chunk_size && !current.is_empty() {
            chunks.push(current.trim().to_string());
            current.clear();
        }
        current.push_str(&piece);
    }

    if !current.trim().is_empty() {
        chunks.push(current.trim().to_string());
    }

    if chunks.is_empty() && !text.trim().is_empty() {
        chunks.push(text.to_string());
    }

    chunks
}
