use rig_demo::{agent::RigAgent, cli_chatbot::cli_chatbot};
use tracing::info;

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();
    tracing_subscriber::fmt::init();
    info!("Starting Agent");

    let agent = RigAgent::from_env().build().await.unwrap();

    if let Err(e) = cli_chatbot(agent.agent.as_ref()).await {
        eprintln!("Error: {}", e);
    }
}
