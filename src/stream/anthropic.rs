use anyhow::{Context, Result};
use futures_util::StreamExt;
use reqwest_eventsource::{Event, EventSource};
use serde::{Deserialize, Serialize};
use std::future::Future;
use tokio::sync::mpsc;

use crate::chat::Message;
use crate::config::ResolvedConfig;

use super::{StreamEvent, StreamOutcome};

#[derive(Debug, Serialize)]
struct AnthropicRequest<'a> {
    model: &'a str,
    messages: Vec<AnthropicMessage>,
    max_tokens: u32,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    stop_sequences: Vec<String>,
}

#[derive(Debug, Serialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct DeltaEvent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    delta: Option<DeltaPayload>,
}

#[derive(Debug, Deserialize)]
struct DeltaPayload {
    #[serde(rename = "type", default)]
    kind: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

/// POST /v1/messages with stream=true. Anthropic's SSE emits typed events
/// (`message_start`, `content_block_start`, `content_block_delta`, ...) where
/// only `content_block_delta` carries text deltas.
pub async fn run(
    client: &reqwest::Client,
    cfg: &ResolvedConfig,
    messages: &[Message],
    tx: mpsc::Sender<StreamEvent>,
    cancel: impl Future<Output = ()>,
) -> Result<StreamOutcome> {
    let url = format!("{}/messages", cfg.base_url);
    let (system, conv) = split_system(messages);

    let body = AnthropicRequest {
        model: &cfg.model,
        messages: conv
            .into_iter()
            .map(|m| AnthropicMessage {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect(),
        max_tokens: cfg.max_tokens.unwrap_or(4096),
        stream: true,
        system,
        temperature: cfg.temperature,
        top_p: cfg.top_p,
        stop_sequences: cfg.stop.clone(),
    };

    let req = client
        .post(&url)
        .header("x-api-key", &cfg.api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body);

    let mut es = EventSource::new(req).context("failed to start Anthropic SSE request")?;
    tokio::pin!(cancel);

    loop {
        tokio::select! {
            biased;
            _ = &mut cancel => {
                es.close();
                return Ok(StreamOutcome::Cancelled);
            }
            ev = es.next() => {
                let Some(ev) = ev else {
                    return Ok(StreamOutcome::Done);
                };
                match ev {
                    Ok(Event::Open) => {}
                    Ok(Event::Message(m)) => {
                        // Anthropic doesn't emit `[DONE]`; it emits `message_stop`.
                        let parsed: DeltaEvent = match serde_json::from_str(&m.data) {
                            Ok(p) => p,
                            Err(_) => continue,
                        };
                        if parsed.kind == "message_stop" {
                            es.close();
                            return Ok(StreamOutcome::Done);
                        }
                        if parsed.kind == "content_block_delta" {
                            if let Some(d) = parsed.delta {
                                if d.kind.as_deref() == Some("text_delta") {
                                    if let Some(t) = d.text {
                                        if !t.is_empty()
                                            && tx.send(StreamEvent::Token(t)).await.is_err()
                                        {
                                            es.close();
                                            return Ok(StreamOutcome::Cancelled);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(reqwest_eventsource::Error::StreamEnded) => {
                        return Ok(StreamOutcome::Done);
                    }
                    Err(e) => {
                        es.close();
                        return Err(format_error(cfg, e).await);
                    }
                }
            }
        }
    }
}

fn split_system(messages: &[Message]) -> (Option<String>, Vec<Message>) {
    let mut system = None;
    let mut conv = Vec::with_capacity(messages.len());
    for m in messages {
        if m.role == "system" {
            // Concatenate multiple system messages with newlines.
            system = Some(match system {
                Some(prev) => format!("{prev}\n{}", m.content),
                None => m.content.clone(),
            });
        } else {
            conv.push(m.clone());
        }
    }
    (system, conv)
}

async fn format_error(cfg: &ResolvedConfig, err: reqwest_eventsource::Error) -> anyhow::Error {
    use reqwest_eventsource::Error as E;
    match err {
        E::InvalidStatusCode(status, resp) => {
            let body = resp.text().await.unwrap_or_default();
            anyhow::anyhow!(
                "Anthropic request to {} failed with HTTP {}\nmodel: {}\nbody: {}",
                cfg.base_url,
                status,
                cfg.model,
                body
            )
        }
        E::Transport(e) => anyhow::anyhow!(
            "transport error contacting {} (anthropic): {}",
            cfg.base_url,
            e
        ),
        other => anyhow::anyhow!("Anthropic stream error: {}", other),
    }
}
