use std::io::{self, Write};

use futures_util::StreamExt;
use rig::agent::Agent;
use rig::completion::{Chat, Message, PromptError};

use rig::streaming::{StreamingChat, StreamingChoice, StreamingCompletionModel, StreamingResult};

/// Utility function to create a simple REPL CLI chatbot from a type that implements the
/// `Chat` trait.
pub async fn cli_chatbot(chatbot: &impl Chat) -> Result<(), PromptError> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut chat_log = vec![];

    println!("Welcome to the chatbot! Type 'exit' to quit.");
    loop {
        print!("> ");
        // Flush stdout to ensure the prompt appears before input
        stdout.flush().unwrap();

        let mut input = String::new();
        match stdin.read_line(&mut input) {
            Ok(_) => {
                // Remove the newline character from the input
                let input = input.trim();
                // Check for a command to exit
                if input == "exit" || input == "bye" {
                    break;
                }
                tracing::info!("Prompt:\n{}\n", input);

                let response = chatbot.chat(input, chat_log.clone()).await?;
                chat_log.push(Message::user(input));
                chat_log.push(Message::assistant(response.clone()));

                // println!("========================== Response ============================");
                // println!("{response}");
                // println!("================================================================\n\n");

                tracing::info!("Response:\n{}\n", response);
            },
            Err(error) => println!("Error reading input: {}", error),
        }
    }

    Ok(())
}

pub async fn cli_chatbot2<M: StreamingCompletionModel>(
    chatbot: Agent<M>,
) -> Result<(), PromptError> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut chat_log = vec![];

    println!("Welcome to the chatbot! Type 'exit' to quit.");
    loop {
        print!("> ");
        // Flush stdout to ensure the prompt appears before input
        stdout.flush().unwrap();

        let mut input = String::new();
        match stdin.read_line(&mut input) {
            Ok(_) => {
                // Remove the newline character from the input
                let input = input.trim();
                // Check for a command to exit
                if input == "exit" || input == "bye" {
                    break;
                }
                tracing::info!("Prompt:\n{}\n", input);

                // let response = chatbot.chat(input, chat_log.clone()).await?;
                chat_log.push(Message::user(input));

                let mut stream = chatbot.stream_chat(input, chat_log.clone()).await?;
                let logs = stream_to_stdout(&chatbot, &mut stream).await.unwrap();
                chat_log.push(Message::assistant(logs));
            },
            Err(error) => println!("Error reading input: {}", error),
        }
    }

    Ok(())
}

/// helper function to stream a completion request to stdout
pub async fn stream_to_stdout<M: StreamingCompletionModel>(
    agent: &Agent<M>, stream: &mut StreamingResult,
) -> Result<String, std::io::Error> {
    let mut logs = String::new();
    print!("Response: ");
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(StreamingChoice::Message(text)) => {
                print!("{}", text);
                logs.push_str(&text);
                std::io::Write::flush(&mut std::io::stdout())?;
            },
            Ok(StreamingChoice::ToolCall(name, _, params)) => {
                let res =
                    agent.tools.call(&name, params.to_string()).await.map_err(|e| {
                        std::io::Error::new(std::io::ErrorKind::Other, e.to_string())
                    })?;
                println!("\nResult: {}", res);
            },
            Err(e) => {
                eprintln!("Error: {}", e);
                break;
            },
        }
    }
    println!(); // New line after streaming completes

    Ok(logs)
}
