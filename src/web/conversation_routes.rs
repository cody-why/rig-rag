use std::{net::SocketAddr, sync::Arc};

use axum::{
    Router,
    extract::{ConnectInfo, Extension, Json, Path, Query, State},
    http::HeaderMap,
    middleware,
    response::Json as ResponseJson,
    routing::{delete, get, post},
};
use serde::{Deserialize, Serialize};
use tracing::{error, info};

use crate::{
    agent::RigAgent,
    db::{
        Conversation, ConversationMessage, ConversationStats, ConversationStatus,
        ConversationStore, CreateMessageRequest, DocumentStore, UserInteractionStats,
    },
    web::auth_routes::{Claims, require_admin_auth_middleware},
};

type AppState = (Arc<RigAgent>, Arc<DocumentStore>, Arc<ConversationStore>);

/// 对话聊天请求
#[derive(Debug, Deserialize)]
pub struct ConversationChatRequest {
    pub message: String,
    pub user_id: Option<String>,
    pub conversation_id: Option<String>,
}

/// 对话聊天响应
#[derive(Debug, Serialize)]
pub struct ConversationChatResponse {
    pub response: String,
    pub user_id: String,
    pub conversation_id: String,
    pub message_id: String,
}

/// 对话历史响应
#[derive(Debug, Serialize)]
pub struct ConversationHistoryResponse {
    pub conversation: Conversation,
    pub messages: Vec<ConversationMessage>,
}

/// 用户对话列表响应
#[derive(Debug, Serialize)]
pub struct UserConversationsResponse {
    pub conversations: Vec<Conversation>,
    pub total: i64,
    pub has_more: bool,
}

/// 更新对话请求
#[derive(Debug, Deserialize)]
pub struct UpdateConversationWebRequest {
    pub status: Option<ConversationStatus>,
    pub title: Option<String>,
}

/// 查询参数
#[derive(Debug, Deserialize)]
pub struct PaginationQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// 管理员查询参数（包含搜索）
#[derive(Debug, Deserialize)]
pub struct AdminPaginationQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub search: Option<String>,
}

pub fn create_conversation_router() -> Router<AppState> {
    let public_routes = Router::new()
        .route(
            "/api/conversation/{conversation_id}",
            get(get_conversation).put(update_conversation),
        )
        .route(
            "/api/conversation/{conversation_id}/messages",
            get(get_conversation_messages).post(add_message_to_conversation),
        )
        .route(
            "/api/user/{user_id}/conversations",
            get(get_user_conversations),
        )
        .route("/api/user/{user_id}/stats", get(get_user_interaction_stats));

    // 删除对话/清理旧记录等写操作：仅 Admin，并记录操作者与 IP
    let admin_routes = Router::new()
        .route(
            "/api/conversation/{conversation_id}",
            delete(delete_conversation),
        )
        .route("/api/admin/conversations", get(get_all_conversations))
        .route(
            "/api/admin/conversations/stats",
            get(get_conversation_stats),
        )
        .route(
            "/api/admin/conversations/cleanup",
            post(cleanup_old_conversations),
        )
        .route_layer(middleware::from_fn(require_admin_auth_middleware));

    public_routes.merge(admin_routes)
}

/// 解析客户端 IP：优先 X-Forwarded-For / X-Real-IP，否则用直连地址
fn client_ip(headers: &HeaderMap, addr: SocketAddr) -> String {
    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok())
        && let Some(first) = xff.split(',').next()
    {
        let ip = first.trim();
        if !ip.is_empty() {
            return ip.to_string();
        }
    }
    if let Some(real) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
        let ip = real.trim();
        if !ip.is_empty() {
            return ip.to_string();
        }
    }
    addr.ip().to_string()
}

/// 获取对话详情
pub async fn get_conversation(
    State((_, _, conversation_store)): State<AppState>,
    Path(conversation_id): Path<String>,
) -> ResponseJson<Option<Conversation>> {
    match conversation_store
        .get_conversation_by_id(&conversation_id)
        .await
    {
        Ok(conversation) => ResponseJson(conversation),
        Err(e) => {
            error!("Failed to get conversation: {}", e);
            ResponseJson(None)
        }
    }
}

/// 更新对话
pub async fn update_conversation(
    State((_, _, conversation_store)): State<AppState>,
    Path(conversation_id): Path<String>,
    Json(payload): Json<UpdateConversationWebRequest>,
) -> ResponseJson<Option<Conversation>> {
    use crate::db::UpdateConversationRequest;
    let req = UpdateConversationRequest {
        status: payload.status,
        title: payload.title,
        metadata: None,
    };

    match conversation_store
        .update_conversation(&conversation_id, req)
        .await
    {
        Ok(conversation) => ResponseJson(Some(conversation)),
        Err(e) => {
            error!("Failed to update conversation: {}", e);
            ResponseJson(None)
        }
    }
}

/// 删除对话（硬删除，含其全部消息）
pub async fn delete_conversation(
    State((_, _, conversation_store)): State<AppState>,
    Extension(claims): Extension<Claims>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
) -> ResponseJson<serde_json::Value> {
    let client_ip = client_ip(&headers, addr);
    info!(
        user_id = claims.user_id,
        username = %claims.sub,
        role = ?claims.role,
        client_ip = %client_ip,
        conversation_id = %conversation_id,
        "Deleting conversation and messages"
    );

    match conversation_store
        .delete_conversation(&conversation_id)
        .await
    {
        Ok(deleted_messages) => {
            info!(
                user_id = claims.user_id,
                username = %claims.sub,
                client_ip = %client_ip,
                conversation_id = %conversation_id,
                deleted_messages,
                "Deleted conversation and messages"
            );
            ResponseJson(serde_json::json!({
                "success": true,
                "message": "Conversation deleted successfully",
                "deleted_messages": deleted_messages,
            }))
        }
        Err(e) => {
            error!(
                user_id = claims.user_id,
                username = %claims.sub,
                client_ip = %client_ip,
                conversation_id = %conversation_id,
                "Failed to delete conversation: {}",
                e
            );
            ResponseJson(serde_json::json!({"success": false, "error": e.to_string()}))
        }
    }
}

/// 获取对话消息
pub async fn get_conversation_messages(
    State((_, _, conversation_store)): State<AppState>,
    Path(conversation_id): Path<String>,
    Query(pagination): Query<PaginationQuery>,
) -> ResponseJson<Vec<ConversationMessage>> {
    match conversation_store
        .get_conversation_messages(&conversation_id, pagination.limit, pagination.offset)
        .await
    {
        Ok(messages) => ResponseJson(messages),
        Err(e) => {
            error!("Failed to get conversation messages: {}", e);
            ResponseJson(Vec::new())
        }
    }
}

/// 添加消息到对话
pub async fn add_message_to_conversation(
    State((_, _, conversation_store)): State<AppState>,
    Path(conversation_id): Path<String>,
    Json(payload): Json<CreateMessageRequest>,
) -> ResponseJson<Option<ConversationMessage>> {
    let req = CreateMessageRequest {
        conversation_id: conversation_id.clone(),
        role: payload.role,
        content: payload.content,
        metadata: payload.metadata,
    };

    match conversation_store.add_message(req).await {
        Ok(message) => ResponseJson(Some(message)),
        Err(e) => {
            error!("Failed to add message to conversation: {}", e);
            ResponseJson(None)
        }
    }
}

/// 获取用户的对话列表
pub async fn get_user_conversations(
    State((_, _, conversation_store)): State<AppState>,
    Path(user_id): Path<String>,
    Query(pagination): Query<PaginationQuery>,
) -> ResponseJson<UserConversationsResponse> {
    match conversation_store
        .get_user_conversations(&user_id, pagination.limit, pagination.offset)
        .await
    {
        Ok(conversations) => {
            let has_more = conversations.len() as i64 == pagination.limit.unwrap_or(20);
            ResponseJson(UserConversationsResponse {
                total: conversations.len() as i64, // 简化实现，实际应该查询总数
                conversations,
                has_more,
            })
        }
        Err(e) => {
            error!("Failed to get user conversations: {}", e);
            ResponseJson(UserConversationsResponse {
                total: 0,
                conversations: Vec::new(),
                has_more: false,
            })
        }
    }
}

/// 获取用户交互统计
pub async fn get_user_interaction_stats(
    State((_, _, conversation_store)): State<AppState>,
    Path(user_id): Path<String>,
) -> ResponseJson<Option<UserInteractionStats>> {
    match conversation_store
        .get_user_interaction_stats(&user_id)
        .await
    {
        Ok(stats) => ResponseJson(Some(stats)),
        Err(e) => {
            error!("Failed to get user interaction stats: {}", e);
            ResponseJson(None)
        }
    }
}

// ==================== 管理员API ====================

/// 清理请求
#[derive(Debug, Deserialize)]
pub struct CleanupRequest {
    pub days_to_keep: i64,
}

/// 获取所有对话（管理员功能）
pub async fn get_all_conversations(
    State((_, _, conversation_store)): State<AppState>,
    Query(pagination): Query<AdminPaginationQuery>,
) -> ResponseJson<UserConversationsResponse> {
    let search_param = pagination.search.as_deref();

    match conversation_store
        .get_all_conversations(pagination.limit, pagination.offset, search_param)
        .await
    {
        Ok(conversations) => {
            let has_more = conversations.len() as i64 == pagination.limit.unwrap_or(20);
            ResponseJson(UserConversationsResponse {
                total: conversations.len() as i64, // 简化实现
                conversations,
                has_more,
            })
        }
        Err(e) => {
            error!("Failed to get all conversations: {}", e);
            ResponseJson(UserConversationsResponse {
                total: 0,
                conversations: Vec::new(),
                has_more: false,
            })
        }
    }
}

/// 获取对话统计信息
pub async fn get_conversation_stats(
    State((_, _, conversation_store)): State<AppState>,
) -> ResponseJson<Option<ConversationStats>> {
    match conversation_store.get_conversation_stats().await {
        Ok(stats) => ResponseJson(Some(stats)),
        Err(e) => {
            error!("Failed to get conversation stats: {}", e);
            ResponseJson(None)
        }
    }
}

/// 清理旧对话记录
pub async fn cleanup_old_conversations(
    State((_, _, conversation_store)): State<AppState>,
    Extension(claims): Extension<Claims>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(payload): Json<CleanupRequest>,
) -> ResponseJson<serde_json::Value> {
    let client_ip = client_ip(&headers, addr);

    if payload.days_to_keep < 1 {
        return ResponseJson(serde_json::json!({
            "success": false,
            "error": "保留天数必须大于0"
        }));
    }

    info!(
        user_id = claims.user_id,
        username = %claims.sub,
        client_ip = %client_ip,
        days_to_keep = payload.days_to_keep,
        "Cleaning up old conversations"
    );

    match conversation_store
        .cleanup_old_data(payload.days_to_keep)
        .await
    {
        Ok(deleted_count) => {
            info!(
                user_id = claims.user_id,
                username = %claims.sub,
                client_ip = %client_ip,
                deleted_count,
                days_to_keep = payload.days_to_keep,
                "Cleaned up old conversations"
            );
            ResponseJson(serde_json::json!({
                "success": true,
                "deleted_count": deleted_count,
                "message": format!("成功清理了 {} 条旧对话记录", deleted_count)
            }))
        }
        Err(e) => {
            error!(
                user_id = claims.user_id,
                username = %claims.sub,
                client_ip = %client_ip,
                "Failed to cleanup old conversations: {}",
                e
            );
            ResponseJson(serde_json::json!({
                "success": false,
                "error": e.to_string()
            }))
        }
    }
}
