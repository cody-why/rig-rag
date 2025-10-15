use std::{sync::Arc, time::Duration};

use axum::{Router, extract::{Json, Path, State}, routing::{get, post}};
use mini_moka::sync::Cache;
use rig::{completion::Message, message::{AssistantContent, UserContent}};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{error, info};

use crate::{agent::RigAgent, db::{ConversationStore, CreateMessageRequest, DocumentStore, MessageRole}};

pub type UserId = String;
pub type ChatHistory = Vec<Message>;
pub type ChatStore = Arc<RwLock<Cache<UserId, ChatHistory>>>;
pub type ChatAppState = (
    Arc<RigAgent>,
    Option<Arc<DocumentStore>>,
    ChatStore,
    Arc<ConversationStore>,
);

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

pub fn create_chat_store() -> ChatStore {
    let cache: Cache<UserId, ChatHistory> = Cache::builder()
        .time_to_idle(Duration::from_secs(30 * 60))
        // .time_to_live(Duration::from_secs(60 * 60))
        .build();
    // 创建聊天历史存储
    Arc::new(RwLock::new(cache))
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
    // if message.chars().count() < 2 {
    //     return true;
    // }

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
    State((agent, _, chat_store, conversation_store)): State<ChatAppState>,
    Json(payload): Json<ChatRequest>,
) -> Json<ChatResponse> {
    // 从请求中获取用户 ID 或生成一个新的
    let user_id = payload.user_id.unwrap_or_else(generate_user_id);
    let message = payload.message.trim();

    info!("Received chat request from user {}: {}", user_id, message);

    // 从内存缓存获取聊天历史
    let chat_history = {
        let chat_store = chat_store.read().await;
        chat_store.get(&user_id).unwrap_or_default()
    };

    // 使用 RigAgent 处理聊天请求
    let response = match agent.dynamic_chat(message, chat_history).await {
        Ok(response) => {
            if !is_meaningless_message(message) {
                // 更新内存缓存
                {
                    let chat_store_write = chat_store.write().await;
                    let mut history = chat_store_write.get(&user_id).unwrap_or_default();

                    // 添加用户消息和 AI 响应到历史记录
                    history.push(Message::user(message));
                    history.push(Message::assistant(&response));

                    // 保存历史记录条数上限
                    if history.len() > 10 {
                        history.remove(0);
                        history.remove(0);
                    }

                    chat_store_write.insert(user_id.clone(), history);
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
    State((_, _, chat_store, _)): State<ChatAppState>, Path(user_id): Path<String>,
) -> Json<Vec<ChatHistoryItem>> {
    let chat_store = chat_store.read().await;

    let history = chat_store.get(&user_id).unwrap_or_default();

    let history_items = history
        .into_iter()
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
}

fn generate_user_id() -> String {
    // use std::time::{SystemTime, UNIX_EPOCH};
    // let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();
    // format!("user_{}", now)
    nanoid::nanoid!()
}
