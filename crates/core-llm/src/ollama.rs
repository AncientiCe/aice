//! Ollama HTTP streaming chat client.

use crate::LlmError;
use async_trait::async_trait;
use core_orchestrator::{LlmCallOptions, LlmStream};
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
    format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<ChatOptions>,
}

#[derive(serde::Serialize)]
struct ChatOptions {
    num_predict: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
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

#[derive(Deserialize)]
struct NonStreamResponse {
    message: Option<StreamMessage>,
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
    const SHORT_REPLY_STYLE_PROMPT: &'static str =
        "You are a private home voice assistant. Reply in 1-2 short sentences unless the user asks for detail.";
    const PLAIN_SPOKEN_TEXT_RULE: &'static str =
        "Output plain spoken text only. Do not use Markdown, bullet points, numbered lists, headings, code fences, tables, or emojis.";
    const USER_OUTPUT_CONTRACT: &'static str =
        "Output contract: respond in plain spoken text only. Never use markdown, bullets, numbered lists, headings, or code blocks.";

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

    fn compose_system_prompt(&self, system_prompt_override: Option<&str>) -> Option<String> {
        if let Some(override_prompt) = system_prompt_override {
            return Some(override_prompt.to_string());
        }
        let base = self
            .system_prompt
            .as_deref()
            .or({
                if self.short_replies {
                    Some(Self::SHORT_REPLY_STYLE_PROMPT)
                } else {
                    None
                }
            })
            .map(str::trim)
            .filter(|s| !s.is_empty());
        Some(match base {
            Some(prompt) => format!("{}\n\n{}", prompt, Self::PLAIN_SPOKEN_TEXT_RULE),
            None => Self::PLAIN_SPOKEN_TEXT_RULE.to_string(),
        })
    }

    fn compose_user_message(
        &self,
        user_text: &str,
        system_prompt_override: Option<&str>,
    ) -> String {
        if system_prompt_override.is_some() {
            return user_text.to_string();
        }
        format!("{}\n\n{}", user_text, Self::USER_OUTPUT_CONTRACT)
    }

    fn build_messages(
        &self,
        user_text: &str,
        history: &[(String, String)],
        system_prompt_override: Option<&str>,
    ) -> Vec<ChatMessage> {
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
        let prompt = self.compose_system_prompt(system_prompt_override);
        if let Some(prompt) = prompt {
            messages.insert(
                0,
                ChatMessage {
                    role: "system".to_string(),
                    content: prompt,
                },
            );
        }
        let user_message = self.compose_user_message(user_text, system_prompt_override);
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: user_message,
        });
        messages
    }

    fn resolve_num_predict(&self, call_options: Option<&LlmCallOptions>) -> u32 {
        call_options
            .and_then(|o| o.max_output_tokens)
            .unwrap_or(self.max_output_tokens)
            .max(16)
    }

    pub async fn chat_once(
        &self,
        user_text: &str,
        history: &[(String, String)],
        system_prompt_override: Option<&str>,
        call_options: Option<&LlmCallOptions>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let messages = self.build_messages(user_text, history, system_prompt_override);
        let format = call_options
            .filter(|o| o.format_json)
            .map(|_| "json".to_string());
        let temperature = call_options.and_then(|o| o.temperature);
        let body = ChatRequest {
            model: self.model.clone(),
            messages,
            stream: false,
            format,
            options: Some(ChatOptions {
                num_predict: self.resolve_num_predict(call_options),
                temperature,
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
        let payload = res
            .json::<NonStreamResponse>()
            .await
            .map_err(|e| LlmError::Request(e.to_string()))?;
        Ok(payload.message.and_then(|m| m.content).unwrap_or_default())
    }
}

#[async_trait]
impl LlmStream for OllamaLlmStream {
    async fn chat_stream(
        &self,
        user_text: &str,
        history: &[(String, String)],
        system_prompt_override: Option<&str>,
        call_options: Option<&LlmCallOptions>,
    ) -> Result<
        Box<dyn Stream<Item = String> + Send + Unpin>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let messages = self.build_messages(user_text, history, system_prompt_override);
        let format = call_options
            .filter(|o| o.format_json)
            .map(|_| "json".to_string());
        let temperature = call_options.and_then(|o| o.temperature);
        let body = ChatRequest {
            model: self.model.clone(),
            messages,
            stream: true,
            format,
            options: Some(ChatOptions {
                num_predict: self.resolve_num_predict(call_options),
                temperature,
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

#[cfg(test)]
mod tests {
    use core_orchestrator::LlmCallOptions;

    pub trait TestOptionExt<T> {
        fn must(self) -> T;
    }

    impl<T> TestOptionExt<T> for Option<T> {
        fn must(self) -> T {
            match self {
                Some(value) => value,
                None => panic!("expected Some(..) in test"),
            }
        }
    }

    use super::OllamaLlmStream;

    #[test]
    fn compose_system_prompt_adds_plain_text_rule_for_short_replies() {
        let llm = OllamaLlmStream::new(
            "http://localhost:11434".to_string(),
            "tiny".to_string(),
            true,
            64,
            Some("You are concise.".to_string()),
        );
        let prompt = llm.compose_system_prompt(None).must();
        assert!(prompt.contains("You are concise."));
        assert!(prompt.contains("Do not use Markdown"));
    }

    #[test]
    fn compose_system_prompt_keeps_override_unchanged() {
        let llm = OllamaLlmStream::new(
            "http://localhost:11434".to_string(),
            "tiny".to_string(),
            true,
            64,
            Some("ignored".to_string()),
        );
        let prompt = llm
            .compose_system_prompt(Some("classification only"))
            .must();
        assert_eq!(prompt, "classification only");
    }

    #[test]
    fn compose_system_prompt_adds_plain_text_rule_even_without_short_replies() {
        let llm = OllamaLlmStream::new(
            "http://localhost:11434".to_string(),
            "tiny".to_string(),
            false,
            64,
            Some("You are helpful.".to_string()),
        );
        let prompt = llm.compose_system_prompt(None).must();
        assert!(prompt.contains("You are helpful."));
        assert!(prompt.contains("Do not use Markdown"));
    }

    #[test]
    fn compose_user_message_adds_voice_contract_without_override() {
        let llm = OllamaLlmStream::new(
            "http://localhost:11434".to_string(),
            "tiny".to_string(),
            true,
            64,
            Some("You are helpful.".to_string()),
        );
        let msg = llm.compose_user_message("tell me something", None);
        assert!(msg.starts_with("tell me something"));
        assert!(msg.contains("Output contract:"));
        assert!(msg.contains("Never use markdown"));
    }

    #[test]
    fn compose_user_message_keeps_original_with_override() {
        let llm = OllamaLlmStream::new(
            "http://localhost:11434".to_string(),
            "tiny".to_string(),
            true,
            64,
            Some("You are helpful.".to_string()),
        );
        let msg = llm.compose_user_message("classify this", Some("override"));
        assert_eq!(msg, "classify this");
    }

    #[test]
    fn resolve_num_predict_prefers_call_override() {
        let llm = OllamaLlmStream::new(
            "http://localhost:11434".to_string(),
            "tiny".to_string(),
            true,
            64,
            Some("You are helpful.".to_string()),
        );
        let options = LlmCallOptions {
            temperature: Some(0.1),
            format_json: true,
            max_output_tokens: Some(24),
        };
        assert_eq!(llm.resolve_num_predict(Some(&options)), 24);
        assert_eq!(llm.resolve_num_predict(None), 64);
    }
}
