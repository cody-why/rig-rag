use std::sync::Arc;

use axum::{Router, extract::{Json, Path, Query, State}, response::Json as ResponseJson, routing::{delete, get, post, put}};
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};

use crate::{agent::RigAgent, db::{Conversation, ConversationMessage, ConversationStats, ConversationStatus, ConversationStore, CreateMessageRequest, DocumentStore, MessageRole, UserInteractionStats}};

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
        .route("/api/conversation/chat", post(handle_conversation_chat))
        .route("/api/conversation/{conversation_id}", get(get_conversation))
        .route(
            "/api/conversation/{conversation_id}",
            put(update_conversation),
        )
        .route(
            "/api/conversation/{conversation_id}",
            delete(delete_conversation),
        )
        .route(
            "/api/conversation/{conversation_id}/messages",
            get(get_conversation_messages),
        )
        .route(
            "/api/conversation/{conversation_id}/messages",
            post(add_message_to_conversation),
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

/// 处理对话聊天请求
pub async fn handle_conversation_chat(
    State((agent, _, conversation_store)): State<AppState>,
    Json(payload): Json<ConversationChatRequest>,
) -> ResponseJson<ConversationChatResponse> {
    // 生成或使用提供的用户ID
    let user_id = payload.user_id.unwrap_or_else(generate_user_id);

    // 获取或创建活跃对话
    let conversation = match conversation_store
        .get_or_create_active_conversation(&user_id)
        .await
    {
        Ok(conv) => conv,
        Err(e) => {
            error!("Failed to get or create conversation: {}", e);
            return ResponseJson(ConversationChatResponse {
                response: "Sorry, I encountered an error processing your request.".to_string(),
                user_id: user_id.clone(),
                conversation_id: "".to_string(),
                message_id: "".to_string(),
            });
        },
    };

    // 如果提供了特定的对话ID，使用该对话
    let conversation_id = payload.conversation_id.unwrap_or(conversation.id.clone());

    // 获取对话历史消息
    let conversation_messages = match conversation_store
        .get_conversation_messages(&conversation_id, Some(20), None)
        .await
    {
        Ok(messages) => messages,
        Err(e) => {
            warn!("Failed to get conversation messages: {}", e);
            Vec::new()
        },
    };

    // 转换为 rig 的 Message 格式
    let chat_history = conversation_messages
        .into_iter()
        .map(|msg| match msg.role {
            MessageRole::User => rig::completion::Message::user(&msg.content),
            MessageRole::Assistant => rig::completion::Message::assistant(&msg.content),
            MessageRole::System => {
                // System 消息暂时跳过，rig 可能不支持
                rig::completion::Message::assistant(&msg.content)
            },
        })
        .collect();

    info!(
        "Processing chat request for conversation {}: {}",
        conversation_id, payload.message
    );

    // 使用 RigAgent 处理聊天请求
    let response = match agent.dynamic_chat(&payload.message, chat_history).await {
        Ok(response) => {
            // 保存用户消息
            let user_message_req = CreateMessageRequest {
                conversation_id: conversation_id.clone(),
                role: MessageRole::User,
                content: payload.message.clone(),
                metadata: None,
            };

            if let Err(e) = conversation_store.add_message(user_message_req).await {
                warn!("Failed to save user message: {}", e);
            }

            // 保存AI响应
            let assistant_message_req = CreateMessageRequest {
                conversation_id: conversation_id.clone(),
                role: MessageRole::Assistant,
                content: response.clone(),
                metadata: None,
            };

            let message_result = conversation_store.add_message(assistant_message_req).await;
            let message_id = match message_result {
                Ok(msg) => msg.id,
                Err(e) => {
                    warn!("Failed to save assistant message: {}", e);
                    nanoid::nanoid!()
                },
            };

            info!(
                "Chat response for conversation {}: {}",
                conversation_id, response
            );
            (response, message_id)
        },
        Err(e) => {
            error!("Error generating chat response: {}", e);
            let error_response = format!("Sorry, I encountered an error: {}", e);

            // 保存错误消息
            let error_message_req = CreateMessageRequest {
                conversation_id: conversation_id.clone(),
                role: MessageRole::System,
                content: format!("Error: {}", e),
                metadata: Some(serde_json::json!({"error": true})),
            };

            let message_result = conversation_store.add_message(error_message_req).await;
            let message_id = match message_result {
                Ok(msg) => msg.id,
                Err(_) => nanoid::nanoid!(),
            };

            (error_response, message_id)
        },
    };

    ResponseJson(ConversationChatResponse {
        response: response.0,
        user_id,
        conversation_id,
        message_id: response.1,
    })
}

/// 获取对话详情
pub async fn get_conversation(
    State((_, _, conversation_store)): State<AppState>, Path(conversation_id): Path<String>,
) -> ResponseJson<Option<Conversation>> {
    match conversation_store
        .get_conversation_by_id(&conversation_id)
        .await
    {
        Ok(conversation) => ResponseJson(conversation),
        Err(e) => {
            error!("Failed to get conversation: {}", e);
            ResponseJson(None)
        },
    }
}

/// 更新对话
pub async fn update_conversation(
    State((_, _, conversation_store)): State<AppState>, Path(conversation_id): Path<String>,
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
        },
    }
}

/// 删除对话（硬删除）
pub async fn delete_conversation(
    State((_, _, conversation_store)): State<AppState>, Path(conversation_id): Path<String>,
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
        },
    }
}

/// 获取对话消息
pub async fn get_conversation_messages(
    State((_, _, conversation_store)): State<AppState>, Path(conversation_id): Path<String>,
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
        },
    }
}

/// 添加消息到对话
pub async fn add_message_to_conversation(
    State((_, _, conversation_store)): State<AppState>, Path(conversation_id): Path<String>,
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
        },
    }
}

/// 获取用户的对话列表
pub async fn get_user_conversations(
    State((_, _, conversation_store)): State<AppState>, Path(user_id): Path<String>,
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
        },
        Err(e) => {
            error!("Failed to get user conversations: {}", e);
            ResponseJson(UserConversationsResponse {
                total: 0,
                conversations: Vec::new(),
                has_more: false,
            })
        },
    }
}

/// 获取用户交互统计
pub async fn get_user_interaction_stats(
    State((_, _, conversation_store)): State<AppState>, Path(user_id): Path<String>,
) -> ResponseJson<Option<UserInteractionStats>> {
    match conversation_store
        .get_user_interaction_stats(&user_id)
        .await
    {
        Ok(stats) => ResponseJson(Some(stats)),
        Err(e) => {
            error!("Failed to get user interaction stats: {}", e);
            ResponseJson(None)
        },
    }
}

/// 生成用户ID
fn generate_user_id() -> String {
    nanoid::nanoid!()
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
        },
        Err(e) => {
            error!("Failed to get all conversations: {}", e);
            ResponseJson(UserConversationsResponse {
                total: 0,
                conversations: Vec::new(),
                has_more: false,
            })
        },
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
        },
    }
}

/// 清理旧对话记录
pub async fn cleanup_old_conversations(
    State((_, _, conversation_store)): State<AppState>, Json(payload): Json<CleanupRequest>,
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
        },
        Err(e) => {
            error!("Failed to cleanup old conversations: {}", e);
            ResponseJson(serde_json::json!({
                "success": false,
                "error": e.to_string()
            }))
        },
    }
}
