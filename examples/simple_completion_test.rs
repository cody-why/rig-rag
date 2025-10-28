#![allow(unused_imports)]
use std::env;

use anyhow::Result;
use rig::{
    agent::stream_to_stdout,
    completion::{Chat, Message, Prompt},
    prelude::CompletionClient,
    providers::openai,
    streaming::StreamingChat,
};

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let openai_api_key = env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set");
    let base_url = env::var("OPENAI_BASE_URL").expect("OPENAI_BASE_URL must be set");
    let model = env::var("OPENAI_MODEL").expect("OPENAI_MODEL must be set");

    println!("🚀 Starting simple completion test...");

    let client = openai::Client::builder(&openai_api_key)
        .base_url(&base_url)
        .build();

    println!("🤖 Testing agent-based completion...");

    let agent = client
        .completion_model(&model)
        .completions_api()
        .into_agent_builder()
        .temperature(0.5)
        .preamble("You are a helpful AI assistant.")
        .max_tokens(1000)
        .build();

    let message = "Hello, please introduce yourself";
    let history = vec![
        Message::user("Previous message"),
        Message::assistant("Previous response"),
    ];

    // match agent.chat(message, history).await {
    //     Ok(response) => {
    //         println!("✅ Chat successful: {}", response);
    //     },
    //     Err(e) => {
    //         println!("❌ Chat failed: {}", e);
    //     },
    // }

    // match agent.prompt(message).await {
    //     Ok(response) => {
    //         println!("✅ Chat successful: {}", response);
    //     },
    //     Err(e) => {
    //         println!("❌ Chat failed: {}", e);
    //     },
    // }

    let mut stream = agent.stream_chat(message, history).await;
    stream_to_stdout(&mut stream).await?;

    Ok(())
}
