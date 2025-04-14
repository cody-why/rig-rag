use axum::{
    Router,
    extract::{Json, Path, State},
    http::{HeaderValue, Method},
    response::Html,
    routing::{get, post},
};
use mini_moka::sync::Cache;
use rig::{
    completion::{Chat, Message},
    message::{AssistantContent, UserContent},
};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;
use tracing::{error, info};

use crate::agent::RigAgent;

type UserId = String;
type ChatHistory = Vec<Message>;
type ChatStore = Arc<RwLock<Cache<UserId, ChatHistory>>>;

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
struct ChatHistoryItem {
    role: String,
    content: String,
}

pub async fn create_router(agent: Arc<RigAgent>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin("http://localhost:3000".parse::<HeaderValue>().unwrap())
        .allow_methods([Method::GET, Method::POST])
        .allow_headers(vec![axum::http::header::CONTENT_TYPE]);

    let cache: Cache<UserId, ChatHistory> = Cache::builder()
        .time_to_idle(Duration::from_secs(30 * 60))
        // .time_to_live(Duration::from_secs(60 * 60))
        .build();
    // 创建聊天历史存储
    let chat_store: ChatStore = Arc::new(RwLock::new(cache));

    Router::new()
        .route("/", get(serve_index))
        .route("/static/{file}", get(static_file))
        .route("/api/chat", post(handle_chat))
        .route("/api/history/{user_id}", get(get_chat_history))
        .layer(
            tower::ServiceBuilder::new()
                .layer(tower_http::limit::RequestBodyLimitLayer::new(1024 * 10)), // 限制消息大小为10KB
        )
        .layer(cors)
        .with_state((agent, chat_store))
}

async fn serve_index() -> Html<String> {
    let file_content = std::fs::read_to_string("static/index.html").unwrap();
    Html(file_content)
}

async fn static_file(Path(file): Path<String>) -> axum::response::Response {
    let file_path = format!("static/{}", file);
    let file_content = std::fs::read_to_string(file_path).unwrap();

    let file_type = file.split(".").last().unwrap();
    let mime_type = match file_type {
        "css" => "text/css",
        "js" => "application/javascript",
        "html" => "text/html",
        "md" => "text/markdown",
        "json" => "application/json",
        "txt" => "text/plain",
        _ => "application/octet-stream",
    };
    // 使用正确的 MIME 类型
    axum::response::Response::builder()
        .header("Content-Type", mime_type)
        .body(axum::body::Body::from(file_content))
        .unwrap()
}

// 简单的语言检测逻辑
fn is_chinese(text: &str) -> bool {
    let chinese_chars = text.chars().filter(|&c| ('\u{4e00}'..='\u{9fff}').contains(&c)).count();
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

async fn handle_chat(
    State((agent, chat_store)): State<(Arc<RigAgent>, ChatStore)>, Json(payload): Json<ChatRequest>,
) -> Json<ChatResponse> {
    // 从请求中获取用户 ID 或生成一个新的
    let user_id = payload.user_id.unwrap_or_else(generate_user_id);
    let message = payload.message.trim();

    info!("Received chat request from user {}: {}", user_id, message);

    // 获取用户的聊天历史
    let chat_history = {
        let chat_store = chat_store.read().await;
        chat_store.get(&user_id).unwrap_or_default()
    };

    // 检测用户输入语言
    let is_chinese_input = is_chinese(message);
    let enhanced_message = if !is_chinese_input {
        // 如果不是中文，添加明确的语言指令
        format!("回复请使用用户的语言,用户的问题是:{}", message)
    } else {
        message.to_string()
    };

    // 使用 RigAgent 处理聊天请求
    let response = match agent.agent.as_ref().chat(enhanced_message, chat_history).await {
        Ok(response) => {
            if !is_meaningless_message(message) {
                // 将用户消息和 AI 响应添加到历史记录
                let chat_store = chat_store.write().await;

                // 获取当前历史记录，如果不存在则创建新的
                let mut history = chat_store.get(&user_id).unwrap_or_default();

                // 添加用户消息和 AI 响应到历史记录
                history.push(Message::user(message));
                history.push(Message::assistant(&response));

                // 如果历史记录超过10条，则删除最早的一条
                if history.len() > 10 {
                    history.remove(0);
                }

                chat_store.insert(user_id.clone(), history);
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

async fn get_chat_history(
    State((_, chat_store)): State<(Arc<RigAgent>, ChatStore)>, Path(user_id): Path<String>,
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
            Message::Assistant { content } => match content.first() {
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
