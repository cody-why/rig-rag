use std::sync::Arc;

use axum::{
    Router,
    extract::{Json, Path, Query, State},
    response::Json as ResponseJson,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use tracing::{error, info};

use crate::{
    agent::RigAgent,
    db::{
        Conversation, ConversationMessage, ConversationStats, ConversationStatus,
        ConversationStore, CreateMessageRequest, DocumentStore, UserInteractionStats,
    },
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
    Router::new()
        .route(
            "/api/conversation/{conversation_id}",
            get(get_conversation)
                .put(update_conversation)
                .delete(delete_conversation),
        )
        .route(
            "/api/conversation/{conversation_id}/messages",
            get(get_conversation_messages).post(add_message_to_conversation),
        )
        .route(
            "/api/user/{user_id}/conversations",
            get(get_user_conversations),
        )
        .route("/api/user/{user_id}/stats", get(get_user_interaction_stats))
        .route("/api/admin/conversations", get(get_all_conversations))
        .route(
            "/api/admin/conversations/stats",
            get(get_conversation_stats),
        )
        .route(
            "/api/admin/conversations/cleanup",
            post(cleanup_old_conversations),
        )
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

/// 删除对话（硬删除）
pub async fn delete_conversation(
    State((_, _, conversation_store)): State<AppState>,
    Path(conversation_id): Path<String>,
) -> ResponseJson<serde_json::Value> {
    match conversation_store
        .delete_conversation(&conversation_id)
        .await
    {
        Ok(_) => ResponseJson(
            serde_json::json!({"success": true, "message": "Conversation deleted successfully"}),
        ),
        Err(e) => {
            error!("Failed to delete conversation: {}", e);
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
    Json(payload): Json<CleanupRequest>,
) -> ResponseJson<serde_json::Value> {
    if payload.days_to_keep < 1 {
        return ResponseJson(serde_json::json!({
            "success": false,
            "error": "保留天数必须大于0"
        }));
    }

    match conversation_store
        .cleanup_old_data(payload.days_to_keep)
        .await
    {
        Ok(deleted_count) => {
            info!("Cleaned up {} old conversations", deleted_count);
            ResponseJson(serde_json::json!({
                "success": true,
                "deleted_count": deleted_count,
                "message": format!("成功清理了 {} 条旧对话记录", deleted_count)
            }))
        }
        Err(e) => {
            error!("Failed to cleanup old conversations: {}", e);
            ResponseJson(serde_json::json!({
                "success": false,
                "error": e.to_string()
            }))
        }
    }
}
