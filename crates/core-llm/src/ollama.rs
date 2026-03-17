//! Ollama HTTP streaming chat client.

use crate::LlmError;
use async_trait::async_trait;
use core_orchestrator::LlmStream;
use futures::Stream;
use futures_util::StreamExt;
use serde::Deserialize;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::sync::mpsc;

#[derive(Deserialize, serde::Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(serde::Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<ChatOptions>,
}

#[derive(serde::Serialize)]
struct ChatOptions {
    num_predict: u32,
}

#[derive(Deserialize)]
struct StreamChunk {
    message: Option<StreamMessage>,
    #[allow(dead_code)]
    done: Option<bool>,
}

#[derive(Deserialize)]
struct StreamMessage {
    content: Option<String>,
}

/// Stream that yields items from an mpsc receiver.
struct ReceiverStream(mpsc::UnboundedReceiver<String>);

impl Stream for ReceiverStream {
    type Item = String;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().0.poll_recv(cx)
    }
}

/// Ollama streaming LLM client (implements LlmStream).
pub struct OllamaLlmStream {
    client: reqwest::Client,
    base_url: String,
    model: String,
    short_replies: bool,
    max_output_tokens: u32,
    system_prompt: Option<String>,
}

impl OllamaLlmStream {
    pub fn new(
        base_url: String,
        model: String,
        short_replies: bool,
        max_output_tokens: u32,
        system_prompt: Option<String>,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            model,
            short_replies,
            max_output_tokens,
            system_prompt,
        }
    }
}

#[async_trait]
impl LlmStream for OllamaLlmStream {
    async fn chat_stream(
        &self,
        user_text: &str,
        history: &[(String, String)],
        system_prompt_override: Option<&str>,
    ) -> Result<
        Box<dyn Stream<Item = String> + Send + Unpin>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let mut messages: Vec<ChatMessage> = history
            .iter()
            .flat_map(|(u, a)| {
                [
                    ChatMessage {
                        role: "user".to_string(),
                        content: u.clone(),
                    },
                    ChatMessage {
                        role: "assistant".to_string(),
                        content: a.clone(),
                    },
                ]
            })
            .collect();
        let prompt = system_prompt_override.or_else(|| {
            self.system_prompt
                .as_deref()
                .or_else(|| {
                    if self.short_replies {
                        Some(
                            "You are a private home voice assistant. Reply in 1-2 short sentences unless the user asks for detail.",
                        )
                    } else {
                        None
                    }
                })
        });
        if let Some(prompt) = prompt {
            messages.insert(
                0,
                ChatMessage {
                    role: "system".to_string(),
                    content: prompt.to_string(),
                },
            );
        }
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: user_text.to_string(),
        });
        let body = ChatRequest {
            model: self.model.clone(),
            messages,
            stream: true,
            options: Some(ChatOptions {
                num_predict: self.max_output_tokens.max(16),
            }),
        };
        let url = format!("{}/api/chat", self.base_url);
        let res = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::Request(e.to_string()))?;
        if !res.status().is_success() {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            return Err(Box::new(LlmError::Request(format!("{}: {}", status, text))));
        }
        let (tx, rx) = mpsc::unbounded_channel::<String>();
        let stream = res.bytes_stream();
        tokio::spawn(async move {
            let mut buf = Vec::new();
            let mut stream = std::pin::pin!(stream);
            while let Some(Ok(chunk)) = stream.next().await {
                buf.extend_from_slice(&chunk);
                while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                    let line: Vec<u8> = buf.drain(..=pos).collect();
                    let line = String::from_utf8_lossy(&line).trim().to_string();
                    if line.is_empty() {
                        continue;
                    }
                    if let Ok(parsed) = serde_json::from_str::<StreamChunk>(&line) {
                        if let Some(msg) = parsed.message.and_then(|m| m.content) {
                            if !msg.is_empty() && tx.send(msg).is_err() {
                                return;
                            }
                        }
                    }
                }
            }
            drop(tx);
        });
        Ok(Box::new(ReceiverStream(rx)))
    }
}
