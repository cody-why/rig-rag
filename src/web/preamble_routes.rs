use axum::{
    Router,
    extract::{Json, State},
    http::StatusCode,
    response::Json as ResponseJson,
    routing::{get, put},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{error, info};

use crate::{agent::RigAgent, db::DocumentStore, web::ChatStore};
use tokio::fs;

#[derive(Debug, Deserialize)]
pub struct UpdatePreambleRequest {
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct PreambleResponse {
    pub content: String,
    pub updated_at: String,
}

pub fn create_preamble_router() -> Router<(Arc<RigAgent>, Option<Arc<DocumentStore>>, ChatStore)> {
    Router::new()
        .route("/api/preamble", get(get_preamble))
        .route("/api/preamble", put(update_preamble))
}

async fn get_preamble(
    State((agent, _, _)): State<(Arc<RigAgent>, Option<Arc<DocumentStore>>, ChatStore)>,
) -> Result<ResponseJson<PreambleResponse>, StatusCode> {
    // 从 agent context 获取 preamble，因为 LanceDB 主要用于向量存储
    if let Ok(context) = agent.context.read() {
        Ok(ResponseJson(PreambleResponse {
            content: context.preamble.clone(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        }))
    } else {
        error!("Failed to read agent context");
        Err(StatusCode::INTERNAL_SERVER_ERROR)
    }
}

async fn update_preamble(
    State((agent, _, _)): State<(Arc<RigAgent>, Option<Arc<DocumentStore>>, ChatStore)>,
    Json(req): Json<UpdatePreambleRequest>,
) -> Result<ResponseJson<PreambleResponse>, StatusCode> {
    // 更新 agent context 中的 preamble 并持久化到文件
    {
        let mut context = agent.context.write().map_err(|_| {
            error!("Failed to write to agent context");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

        context.preamble = req.content.clone();
        context.needs_rebuild = true; // 标记需要重建agent
    } // 释放锁

    // 先同步保存到文件，确保持久化成功
    save_preamble_to_file(&req.content).await.map_err(|e| {
        error!("Failed to save preamble to file: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    info!("✅ Preamble updated in memory and saved to file, agent will be rebuilt on next chat");

    Ok(ResponseJson(PreambleResponse {
        content: req.content,
        updated_at: chrono::Utc::now().to_rfc3339(),
    }))
}

/// 保存 Preamble 到文件
async fn save_preamble_to_file(content: &str) -> Result<(), std::io::Error> {
    let preamble_path =
        std::env::var("PREAMBLE_FILE").unwrap_or_else(|_| "data/preamble.txt".to_string());

    // 确保目录存在
    if let Some(parent) = std::path::Path::new(&preamble_path).parent() {
        fs::create_dir_all(parent).await?;
    }

    // 写入文件
    fs::write(&preamble_path, content).await?;
    info!("Preamble saved to file: {}", preamble_path);
    Ok(())
}
