//! Several Ollama hosts: requests rotate across them and skip a host that fails.

use std::convert::Infallible;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use core_llm::CradleLlmStream;
use core_orchestrator::LlmStream;
use futures::StreamExt;
use http_body_util::{BodyExt, Full};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;

/// A minimal Ollama-compatible `/api/chat` that answers with its own name.
struct Host {
    url: String,
    calls: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Host {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn host(name: &'static str) -> Host {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|error| panic!("bind: {error}"));
    let addr = listener
        .local_addr()
        .unwrap_or_else(|error| panic!("addr: {error}"));
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let task = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let counter = Arc::clone(&counter);
            tokio::spawn(async move {
                let service = service_fn(move |request: Request<hyper::body::Incoming>| {
                    let counter = Arc::clone(&counter);
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                        let path = request.uri().path().to_string();
                        let sent = request
                            .into_body()
                            .collect()
                            .await
                            .map(|collected| collected.to_bytes())
                            .unwrap_or_default();
                        let streaming =
                            !String::from_utf8_lossy(&sent).contains("\"stream\":false");
                        let body = if path == "/api/generate" {
                            "{\"response\":\"\",\"done\":true}\n".to_string()
                        } else if !streaming {
                            format!(
                                "{{\"message\":{{\"role\":\"assistant\",\"content\":\"{name}\"}},\"done\":true}}"
                            )
                        } else {
                            format!(
                                "{{\"message\":{{\"role\":\"assistant\",\"content\":\"{name}\"}},\"done\":false}}\n{{\"message\":{{\"role\":\"assistant\",\"content\":\"\"}},\"done\":true}}\n"
                            )
                        };
                        Ok::<_, Infallible>(Response::new(Full::new(bytes::Bytes::from(body))))
                    }
                });
                let _ = http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    Host {
        url: format!("http://{addr}"),
        calls,
        task,
    }
}

fn client(urls: Vec<String>) -> CradleLlmStream {
    CradleLlmStream::new_with_hosts(urls, "test-model".to_string(), true, 32, None, None)
}

async fn answer(llm: &CradleLlmStream) -> String {
    let mut stream = llm
        .chat_stream("hello", &[], None, None)
        .await
        .unwrap_or_else(|error| panic!("chat: {error}"));
    let mut text = String::new();
    while let Some(token) = stream.next().await {
        text.push_str(&token);
    }
    text
}

#[tokio::test]
async fn requests_rotate_across_healthy_hosts() {
    let a = host("alpha").await;
    let b = host("beta").await;
    let llm = client(vec![a.url.clone(), b.url.clone()]);
    for _ in 0..6 {
        let text = answer(&llm).await;
        assert!(text == "alpha" || text == "beta", "{text}");
    }
    assert_eq!(a.calls.load(Ordering::SeqCst), 3);
    assert_eq!(b.calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn a_dead_host_is_skipped_and_then_avoided() {
    let live = host("live").await;
    let dead = "http://127.0.0.1:1".to_string();
    let llm = client(vec![dead, live.url.clone()]);
    for _ in 0..4 {
        assert_eq!(answer(&llm).await, "live");
    }
    let once = llm
        .chat_once("hello", &[], None, None)
        .await
        .unwrap_or_else(|error| panic!("chat_once: {error}"));
    assert_eq!(once, "live");
    assert!(llm.warm_up().await.is_ok());
    assert_eq!(live.calls.load(Ordering::SeqCst), 6);
}

#[tokio::test]
async fn all_hosts_down_is_an_error() {
    let llm = client(vec![
        "http://127.0.0.1:1".to_string(),
        "http://127.0.0.1:2".to_string(),
    ]);
    assert!(llm.chat_stream("hello", &[], None, None).await.is_err());
}
