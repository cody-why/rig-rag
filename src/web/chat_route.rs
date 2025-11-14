use std::sync::Arc;

use axum::{
    Router,
    extract::{Json, Path, State},
    response::sse::{Event, Sse},
    routing::{get, post},
};
use futures::StreamExt;
use parking_lot::RwLock;
use rig::{
    completion::Message,
    message::{AssistantContent, UserContent},
};
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{error, info};

use crate::{
    agent::RigAgent,
    db::{ConversationStore, CreateMessageRequest, DocumentStore, MessageRole},
    web::chat_store,
};

pub type ChatAppState = (Arc<RigAgent>, Arc<DocumentStore>, Arc<ConversationStore>);

// 配置常量
const COMPRESS_THRESHOLD: usize = 5; // 自动总结条数阈值
const MAX_HISTORY_MESSAGES: usize = 10; // 历史记录最大条数

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
        .route("/api/chat/stream", post(handle_stream_chat))
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

fn is_meaningless_message(msg: &Message) -> bool {
    let is_meaningless = |text: &String| {
        let trimmed = text.trim();
        // 太短或者全是标点符号
        trimmed.len() < 2 || trimmed.chars().all(|c| c.is_ascii_punctuation())
    };

    match msg {
        Message::User { content } => match content.first() {
            UserContent::Text(text) => is_meaningless(&text.text),
            _ => false,
        },
        Message::Assistant { content, .. } => match content.first() {
            AssistantContent::Text(text) => is_meaningless(&text.text),
            _ => false,
        },
    }
}

/// 过滤无意义的历史消息
fn filter_meaningless_messages(history: Vec<Message>) -> Vec<Message> {
    history
        .into_iter()
        .filter(|msg| !is_meaningless_message(msg))
        .collect()
}

/// 压缩历史记录：当历史记录超过阈值时，总结旧消息
/// 返回压缩后的历史记录
async fn compress_history(
    agent: Arc<RigAgent>,
    history: Vec<Message>,
    max_messages: usize,
) -> anyhow::Result<Vec<Message>> {
    // 如果历史记录数量未超过阈值，直接返回
    if history.len() <= max_messages {
        return Ok(history);
    }
    let history_len = history.len();

    // 直接传递 history 给 agent 进行总结
    let summary_prompt = "请简洁地总结以下对话历史，保留关键信息和上下文。";

    // 使用 agent 总结旧消息，直接将 history 作为历史传递
    let summary = match agent.chat(summary_prompt, history.clone()).await {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to summarize history: {}", e);
            // 如果总结失败，返回原始历史
            let skip_len = history_len - max_messages;
            return Ok(history.into_iter().skip(skip_len).collect());
        }
    };

    // 使用用户消息存储总结（标记为系统消息）
    let summary_message = Message::user(format!("[历史总结] {}", summary));

    // 组合：总结消息 + 最近的消息
    let compressed = vec![summary_message];

    info!(
        "Compressed history: {} messages -> {} messages (summary)",
        history_len,
        compressed.len()
    );

    Ok(compressed)
}

/// 自动压缩历史记录
async fn auto_compress_history(
    agent: &Arc<RigAgent>,
    chat_history: &Arc<RwLock<Vec<Message>>>,
    history_to_compress: Vec<Message>,
) -> anyhow::Result<()> {
    match compress_history(agent.clone(), history_to_compress, COMPRESS_THRESHOLD).await {
        Ok(compressed) => {
            let mut history = chat_history.write();
            *history = compressed;
            info!(
                "Auto-compressed chat history after adding new messages (threshold: {})",
                COMPRESS_THRESHOLD
            );
            Ok(())
        }
        Err(e) => {
            error!("Failed to compress history after adding messages: {}", e);
            // 如果压缩失败，至少截断到阈值
            let mut history = chat_history.write();
            let excess = history.len().saturating_sub(COMPRESS_THRESHOLD);
            if excess > 0 {
                history.drain(0..excess);
            }
            Err(e)
        }
    }
}

/// 保存消息到数据库
async fn save_messages_to_db(
    conversation_store: &Arc<ConversationStore>,
    user_id: &str,
    user_message: &str,
    assistant_response: &str,
) {
    let conversation = match conversation_store
        .get_or_create_active_conversation(user_id)
        .await
    {
        Ok(conv) => conv,
        Err(e) => {
            error!("Failed to get or create conversation for DB storage: {}", e);
            return;
        }
    };

    // 保存用户消息到数据库
    let user_message_req = CreateMessageRequest {
        conversation_id: conversation.id.clone(),
        role: MessageRole::User,
        content: user_message.to_string(),
        metadata: None,
    };

    if let Err(e) = conversation_store.add_message(user_message_req).await {
        error!("Failed to save user message to database: {}", e);
    }

    if let Err(e) = conversation_store
        .smart_close_conversation_if_needed(&conversation.id, user_message)
        .await
    {
        error!("Failed to check smart close: {}", e);
    }

    // 保存AI响应到数据库
    let assistant_message_req = CreateMessageRequest {
        conversation_id: conversation.id.clone(),
        role: MessageRole::Assistant,
        content: assistant_response.to_string(),
        metadata: None,
    };

    if let Err(e) = conversation_store.add_message(assistant_message_req).await {
        error!("Failed to save assistant message to database: {}", e);
    }
}

pub async fn handle_chat(
    State((agent, _, conversation_store)): State<ChatAppState>,
    Json(payload): Json<ChatRequest>,
) -> Json<ChatResponse> {
    // 从请求中获取用户 ID 或生成一个新的
    let user_id = payload.user_id.unwrap_or_else(generate_user_id);
    let message = payload.message.trim();

    info!("Received chat request from user {}: {}", user_id, message);

    // 从内存缓存获取或初始化聊天历史
    let chat_history = if let Some(h) = chat_store().get(&user_id) {
        h
    } else {
        let h = Arc::new(RwLock::new(Vec::new()));
        chat_store().insert(user_id.clone(), h.clone());
        h
    };

    // 使用 RigAgent 处理聊天请求
    let history_snapshot = { chat_history.read().clone() };

    let response = match agent.chat(message, history_snapshot).await {
        Ok(response) => {
            // 更新内存缓存（所有消息都保存）
            {
                let mut history = chat_history.write();
                history.push(Message::user(message));
                history.push(Message::assistant(&response));
                // 保存历史记录条数上限
                let excess = history.len().saturating_sub(MAX_HISTORY_MESSAGES);
                if excess > 0 {
                    history.drain(0..excess);
                }
            }

            // 保存消息到数据库
            save_messages_to_db(&conversation_store, &user_id, message, &response).await;

            info!("Chat response for user {}: {}", user_id, response);
            response
        }
        Err(e) => {
            error!("Error generating chat response: {}", e);
            format!("Sorry, I encountered an error: {}", e)
        }
    };

    Json(ChatResponse { response, user_id })
}

/// 流式聊天处理器
pub async fn handle_stream_chat(
    State((agent, _, conversation_store)): State<ChatAppState>,
    Json(payload): Json<ChatRequest>,
) -> Sse<impl futures::Stream<Item = Result<Event, axum::Error>>> {
    // 从请求中获取用户 ID 或生成一个新的
    let no_id = payload.user_id.is_none();
    let user_id = payload.user_id.unwrap_or_else(generate_user_id);
    let message = payload.message.trim().to_string();

    info!(
        "Received stream chat request from user {}: {}",
        user_id, message
    );

    // 从内存缓存获取或初始化聊天历史
    let chat_history = if let Some(h) = chat_store().get(&user_id) {
        h
    } else {
        let h = Arc::new(RwLock::new(Vec::new()));
        chat_store().insert(user_id.clone(), h.clone());
        h
    };

    // 获取用户历史，过滤无意义消息并压缩
    let raw_history = chat_history.read().clone();
    let history_snapshot = filter_meaningless_messages(raw_history);

    // 创建流式响应
    let (tx, rx) = tokio::sync::mpsc::channel(128);

    // 在后台任务中处理流
    let user_id_clone = user_id.clone();
    let message_clone = message.clone();
    let chat_history_clone = chat_history.clone();
    let conversation_store_clone = conversation_store.clone();
    let agent_clone = agent.clone();

    tokio::spawn(async move {
        match agent_clone
            .stream_chat(&message_clone, history_snapshot)
            .await
        {
            Ok(mut stream) => {
                let mut full_response = String::with_capacity(2048);

                if no_id {
                    let _ = tx
                        .send(Ok(Event::default().event("user_id").data(user_id)))
                        .await;
                }

                while let Some(chunk) = stream.next().await {
                    full_response.push_str(&chunk);
                    let chunk = chunk.replace("\n", "[LF]");
                    let _ = tx.send(Ok(Event::default().data(chunk))).await;
                }

                // 发送完成信号
                let _ = tx.send(Ok(Event::default().data("[DONE]"))).await;

                // 更新内存缓存（所有消息都保存）
                {
                    let history_to_compress = {
                        let mut history = chat_history_clone.write();
                        history.push(Message::user(&message_clone));
                        history.push(Message::assistant(&full_response));
                        history.clone()
                    };

                    // 每3条消息自动总结（在锁外进行异步操作）
                    if history_to_compress.len() > COMPRESS_THRESHOLD
                        && let Err(e) = auto_compress_history(
                            &agent_clone,
                            &chat_history_clone,
                            history_to_compress,
                        )
                        .await
                    {
                        error!("Failed to auto-compress history: {}", e);
                    }
                }

                // 保存消息到数据库
                save_messages_to_db(
                    &conversation_store_clone,
                    &user_id_clone,
                    &message_clone,
                    &full_response,
                )
                .await;
            }
            Err(e) => {
                error!("Error creating stream chat: {}", e);
                let _ = tx
                    .send(Ok(Event::default().data(format!("Error: {}", e))))
                    .await;
            }
        }
    });

    Sse::new(ReceiverStream::new(rx)).keep_alive(axum::response::sse::KeepAlive::default())
}

pub async fn get_chat_history(
    State((_, _, _)): State<ChatAppState>,
    Path(user_id): Path<String>,
) -> Json<Vec<ChatHistoryItem>> {
    // 获取或初始化
    if let Some(h) = chat_store().get(&user_id) {
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
