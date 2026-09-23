//! Cradle in-process LLM stream adapter.
//!
//! Transport is the Ollama-compatible API. With several hosts, calls rotate
//! across them; a host that fails is skipped for a while and the call moves
//! to the next one, so one busy or crashed machine does not silence the rooms.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::ollama::OllamaLlmStream;
use async_trait::async_trait;
use core_observability::record_backend_inference_host_error;
use core_orchestrator::{LlmCallOptions, LlmStream};
use futures::Stream;

type DynError = Box<dyn std::error::Error + Send + Sync>;

/// How long a failed host is skipped before it is tried again.
const HOST_COOLDOWN: Duration = Duration::from_secs(30);

struct Host {
    url: String,
    client: OllamaLlmStream,
    down_until: Mutex<Option<Instant>>,
}

impl Host {
    fn is_down(&self) -> bool {
        self.down_until
            .lock()
            .map(|until| until.is_some_and(|until| until > Instant::now()))
            .unwrap_or(false)
    }

    fn mark_down(&self) {
        if let Ok(mut until) = self.down_until.lock() {
            *until = Some(Instant::now() + HOST_COOLDOWN);
        }
        record_backend_inference_host_error(&self.url);
    }

    fn mark_up(&self) {
        if let Ok(mut until) = self.down_until.lock() {
            *until = None;
        }
    }
}

/// Cradle provider implementation used by backend runtime.
pub struct CradleLlmStream {
    hosts: Vec<Host>,
    next: AtomicUsize,
}

impl CradleLlmStream {
    pub fn new(
        base_url: String,
        model: String,
        short_replies: bool,
        max_output_tokens: u32,
        system_prompt: Option<String>,
        keep_alive: Option<String>,
    ) -> Self {
        Self::new_with_hosts(
            vec![base_url],
            model,
            short_replies,
            max_output_tokens,
            system_prompt,
            keep_alive,
        )
    }

    /// One client per host URL; calls rotate across them with failover.
    pub fn new_with_hosts(
        base_urls: Vec<String>,
        model: String,
        short_replies: bool,
        max_output_tokens: u32,
        system_prompt: Option<String>,
        keep_alive: Option<String>,
    ) -> Self {
        let hosts = base_urls
            .into_iter()
            .map(|url| Host {
                client: OllamaLlmStream::new(
                    url.clone(),
                    model.clone(),
                    short_replies,
                    max_output_tokens,
                    system_prompt.clone(),
                    keep_alive.clone(),
                ),
                url,
                down_until: Mutex::new(None),
            })
            .collect();
        Self {
            hosts,
            next: AtomicUsize::new(0),
        }
    }

    /// Hosts to try for one call: rotation order, healthy hosts first,
    /// hosts in cooldown last (better a slow answer than none).
    fn attempt_order(&self) -> Vec<&Host> {
        let count = self.hosts.len();
        if count == 0 {
            return Vec::new();
        }
        let start = self.next.fetch_add(1, Ordering::Relaxed) % count;
        let rotated = (0..count).map(|offset| &self.hosts[(start + offset) % count]);
        let (healthy, cooling): (Vec<&Host>, Vec<&Host>) =
            rotated.partition(|host| !host.is_down());
        healthy.into_iter().chain(cooling).collect()
    }

    pub async fn chat_once(
        &self,
        user_text: &str,
        history: &[(String, String)],
        system_prompt_override: Option<&str>,
        call_options: Option<&LlmCallOptions>,
    ) -> Result<String, DynError> {
        let mut last_error: DynError = "no LLM hosts configured".into();
        for host in self.attempt_order() {
            match host
                .client
                .chat_once(user_text, history, system_prompt_override, call_options)
                .await
            {
                Ok(text) => {
                    host.mark_up();
                    return Ok(text);
                }
                Err(error) => {
                    host.mark_down();
                    tracing::warn!(host = %host.url, %error, "LLM host failed; trying the next one");
                    last_error = error;
                }
            }
        }
        Err(last_error)
    }

    /// Warm every host; succeeds when at least one is ready.
    pub async fn warm_up(&self) -> Result<(), DynError> {
        let mut last_error: Option<DynError> = None;
        let mut any = false;
        for host in &self.hosts {
            match host.client.warm_up().await {
                Ok(()) => {
                    host.mark_up();
                    any = true;
                }
                Err(error) => {
                    host.mark_down();
                    last_error = Some(error);
                }
            }
        }
        match (any, last_error) {
            (true, _) => Ok(()),
            (false, Some(error)) => Err(error),
            (false, None) => Err("no LLM hosts configured".into()),
        }
    }
}

#[async_trait]
impl LlmStream for CradleLlmStream {
    async fn chat_stream(
        &self,
        user_text: &str,
        history: &[(String, String)],
        system_prompt_override: Option<&str>,
        call_options: Option<&LlmCallOptions>,
    ) -> Result<Box<dyn Stream<Item = String> + Send + Unpin>, DynError> {
        let mut last_error: DynError = "no LLM hosts configured".into();
        for host in self.attempt_order() {
            match host
                .client
                .chat_stream(user_text, history, system_prompt_override, call_options)
                .await
            {
                Ok(stream) => {
                    host.mark_up();
                    return Ok(stream);
                }
                Err(error) => {
                    host.mark_down();
                    tracing::warn!(host = %host.url, %error, "LLM host failed; trying the next one");
                    last_error = error;
                }
            }
        }
        Err(last_error)
    }
}
