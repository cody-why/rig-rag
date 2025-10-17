use std::{sync::Arc, sync::OnceLock, time::Duration};

use mini_moka::sync::Cache;
use parking_lot::RwLock;
use rig::completion::Message;

pub type UserId = String;
pub type ChatHistory = Arc<RwLock<Vec<Message>>>;
pub type ChatStore = Cache<UserId, ChatHistory>;

pub fn chat_store() -> &'static ChatStore {
    static CACHE: OnceLock<ChatStore> = OnceLock::new();
    CACHE.get_or_init(|| {
        Cache::builder()
            .time_to_idle(Duration::from_secs(30 * 60))
            // .time_to_live(Duration::from_secs(60 * 60))
            .build()
    })
}
