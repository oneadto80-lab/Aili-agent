use aili::chat::Message;
use aili::config::ResolvedConfig;
use aili::provider::Provider;
use aili::stream::{StreamEvent, StreamOutcome, run_stream};
use std::time::Duration;
use tokio::sync::mpsc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn cfg(server: &MockServer, model: &str) -> ResolvedConfig {
    ResolvedConfig {
        provider: Provider::Vllm,
        base_url: server.uri(),
        api_key: "test".into(),
        model: model.into(),
        temperature: None,
        top_p: None,
        max_tokens: None,
        stop: vec![],
    }
}

fn sse_body(deltas: &[&str]) -> String {
    let mut out = String::new();
    for d in deltas {
        let payload = serde_json::json!({
            "choices": [{ "delta": { "content": d }, "finish_reason": null }]
        });
        out.push_str("data: ");
        out.push_str(&payload.to_string());
        out.push_str("\n\n");
    }
    out.push_str("data: [DONE]\n\n");
    out
}

async fn collect(mut rx: mpsc::Receiver<StreamEvent>) -> Vec<String> {
    let mut tokens = Vec::new();
    while let Some(StreamEvent::Token(t)) = rx.recv().await {
        tokens.push(t);
    }
    tokens
}

#[tokio::test]
async fn streams_tokens_in_order() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_raw(sse_body(&["hel", "lo", " world"]).into_bytes(), "text/event-stream"),
        )
        .mount(&server)
        .await;

    let client = reqwest::Client::new();
    let cfg = cfg(&server, "test-model");
    let messages = vec![Message::user("hi")];
    let (tx, rx) = mpsc::channel(64);
    let drain = tokio::spawn(collect(rx));
    let outcome = run_stream(&client, &cfg, &messages, tx, std::future::pending::<()>())
        .await
        .expect("stream ok");
    assert_eq!(outcome, StreamOutcome::Done);
    let tokens = drain.await.unwrap();
    assert_eq!(tokens, vec!["hel", "lo", " world"]);
    assert_eq!(tokens.concat(), "hello world");
}

#[tokio::test]
async fn http_401_surfaces_status_and_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(401).set_body_string(r#"{"error":{"message":"bad key"}}"#),
        )
        .mount(&server)
        .await;

    let client = reqwest::Client::new();
    let cfg = cfg(&server, "test-model");
    let messages = vec![Message::user("hi")];
    let (tx, rx) = mpsc::channel(64);
    let drain = tokio::spawn(collect(rx));
    let err = run_stream(&client, &cfg, &messages, tx, std::future::pending::<()>())
        .await
        .unwrap_err();
    drain.abort();
    let s = format!("{err:#}");
    assert!(s.contains("401"), "missing status in: {s}");
    assert!(s.contains("bad key"), "missing body in: {s}");
}

#[tokio::test]
async fn cancel_aborts_mid_stream() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_delay(Duration::from_secs(5))
                .set_body_raw(sse_body(&["never"]).into_bytes(), "text/event-stream"),
        )
        .mount(&server)
        .await;

    let client = reqwest::Client::new();
    let cfg = cfg(&server, "test-model");
    let messages = vec![Message::user("hi")];
    let (tx, rx) = mpsc::channel(64);
    let drain = tokio::spawn(collect(rx));
    let cancel = async {
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let outcome = run_stream(&client, &cfg, &messages, tx, cancel)
        .await
        .expect("stream ok");
    assert_eq!(outcome, StreamOutcome::Cancelled);
    let tokens = drain.await.unwrap();
    assert!(tokens.is_empty(), "got tokens during cancel: {tokens:?}");
}
