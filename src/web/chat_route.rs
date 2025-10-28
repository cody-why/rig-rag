use std::sync::Arc;

use axum::{Router, extract::{Json, Path, State}, routing::{get, post}};
use parking_lot::RwLock;
use rig::{completion::Message, message::{AssistantContent, UserContent}};
use serde::{Deserialize, Serialize};
use tracing::{error, info};

use crate::{agent::RigAgent, db::{ConversationStore, CreateMessageRequest, DocumentStore, MessageRole}, web::chat_store};

pub type ChatAppState = (Arc<RigAgent>, Arc<DocumentStore>, Arc<ConversationStore>);

#[derive(Debug, Deserialize)]
pub struct ChatRequest {
    message: String,
    user_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ChatResponse {
    response: String,
    user_id: String,
}

#[derive(Debug, Serialize)]
pub struct ChatHistoryItem {
    role: String,
    content: String,
}

pub fn create_chat_router() -> Router<ChatAppState> {
    Router::new()
        .route("/api/chat", post(handle_chat))
        .route("/api/history/{user_id}", get(get_chat_history))
}

// 简单的语言检测逻辑
#[allow(dead_code)]
fn is_chinese(text: &str) -> bool {
    let chinese_chars = text
        .chars()
        .filter(|&c| ('\u{4e00}'..='\u{9fff}').contains(&c))
        .count();
    let total_chars = text.chars().filter(|&c| !c.is_whitespace()).count();

    // 如果中文字符超过非空白字符的30%，认为是中文
    if total_chars > 0 {
        return chinese_chars as f32 / total_chars as f32 > 0.3;
    }
    true // 默认为中文
}

// 检查是否为无意义短句
fn is_meaningless_message(message: &str) -> bool {
    // 检查字符数
    if message.chars().count() < 2 {
        return true;
    }

    // 检查是否全是标点符号
    if message.chars().all(|c| c.is_ascii_punctuation()) {
        return true;
    }

    // 检查是否重复单字
    // if message.chars().count() == 2 && message.chars().next() == message.chars().nth(1) {
    //     return true;
    // }

    false
}

pub async fn handle_chat(
    State((agent, _, conversation_store)): State<ChatAppState>, Json(payload): Json<ChatRequest>,
) -> Json<ChatResponse> {
    // 从请求中获取用户 ID 或生成一个新的
    let user_id = payload.user_id.unwrap_or_else(generate_user_id);
    let message = payload.message.trim();

    info!("Received chat request from user {}: {}", user_id, message);

    // 从内存缓存获取或初始化聊天历史（使用 RwLock 包装）
    let chat_history = if let Some(h) = chat_store().get(&user_id) {
        h
    } else {
        let h = Arc::new(RwLock::new(Vec::new()));
        chat_store().insert(user_id.clone(), h.clone());
        // 关闭历史对话
        let _ = conversation_store.close_conversation(&user_id).await;
        h
    };

    // 使用 RigAgent 处理聊天请求
    let history_snapshot = { chat_history.read().clone() };
    let response = match agent.dynamic_chat(message, history_snapshot).await {
        Ok(response) => {
            if !is_meaningless_message(message) {
                // 更新内存缓存
                {
                    let history_arc = chat_history.clone();
                    let mut history = history_arc.write();
                    history.push(Message::user(message));
                    history.push(Message::assistant(&response));
                    // 保存历史记录条数上限（最多保留最近 10 条）
                    let excess = history.len().saturating_sub(10);
                    if excess > 0 {
                        for _ in 0..excess {
                            if history.is_empty() {
                                break;
                            }
                            let _ = history.remove(0);
                        }
                    }
                }

                // 获取或创建活跃对话用于数据库存储
                let conversation = match conversation_store
                    .get_or_create_active_conversation(&user_id)
                    .await
                {
                    Ok(conv) => conv,
                    Err(e) => {
                        error!("Failed to get or create conversation for DB storage: {}", e);
                        return Json(ChatResponse { response, user_id });
                    },
                };

                // 保存用户消息到数据库
                let user_message_req = CreateMessageRequest {
                    conversation_id: conversation.id.clone(),
                    role: MessageRole::User,
                    content: message.to_string(),
                    metadata: None,
                };

                if let Err(e) = conversation_store.add_message(user_message_req).await {
                    error!("Failed to save user message to database: {}", e);
                }

                if let Err(e) = conversation_store
                    .smart_close_conversation_if_needed(&conversation.id, message)
                    .await
                {
                    error!("Failed to check smart close: {}", e);
                }

                // 保存AI响应到数据库
                let assistant_message_req = CreateMessageRequest {
                    conversation_id: conversation.id.clone(),
                    role: MessageRole::Assistant,
                    content: response.clone(),
                    metadata: None,
                };

                if let Err(e) = conversation_store.add_message(assistant_message_req).await {
                    error!("Failed to save assistant message to database: {}", e);
                }
            }

            info!("Chat response for user {}: {}", user_id, response);
            response
        },
        Err(e) => {
            error!("Error generating chat response: {}", e);
            format!("Sorry, I encountered an error: {}", e)
        },
    };

    Json(ChatResponse { response, user_id })
}

pub async fn get_chat_history(
    State((_, _, _)): State<ChatAppState>, Path(user_id): Path<String>,
) -> Json<Vec<ChatHistoryItem>> {
    // 获取或初始化
    let history = chat_store().get(&user_id);
    if let Some(h) = history {
        let history_items = h
            .read()
            .iter()
            .filter_map(|msg| match msg {
                Message::User { content } => match content.first() {
                    UserContent::Text(text) => Some(ChatHistoryItem {
                        role: "user".to_string(),
                        content: text.text.clone(),
                    }),
                    _ => None,
                },
                Message::Assistant { id: _, content } => match content.first() {
                    AssistantContent::Text(text) => Some(ChatHistoryItem {
                        role: "assistant".to_string(),
                        content: text.text.clone(),
                    }),
                    _ => None,
                },
            })
            .collect();

        Json(history_items)
    } else {
        Json(Vec::new())
    }
}

fn generate_user_id() -> String {
    // use std::time::{SystemTime, UNIX_EPOCH};
    // let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();
    // format!("user_{}", now)
    nanoid::nanoid!()
}
