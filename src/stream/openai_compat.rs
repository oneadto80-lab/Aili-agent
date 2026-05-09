use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use reqwest_eventsource::{Event, EventSource};
use std::future::Future;
use tokio::sync::mpsc;

use crate::chat::{ChatRequest, Message};
use crate::config::ResolvedConfig;

use super::{StreamEvent, StreamOutcome};

/// POST /v1/chat/completions with stream=true and parse OpenAI-style SSE.
pub async fn run(
    client: &reqwest::Client,
    cfg: &ResolvedConfig,
    messages: &[Message],
    tx: mpsc::Sender<StreamEvent>,
    cancel: impl Future<Output = ()>,
) -> Result<StreamOutcome> {
    let url = format!("{}/chat/completions", cfg.base_url);
    let body = ChatRequest::build(cfg, messages);

    let req = client.post(&url).bearer_auth(&cfg.api_key).json(&body);

    let mut es = EventSource::new(req).context("failed to start SSE request")?;
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
                        if m.data.trim() == "[DONE]" {
                            es.close();
                            return Ok(StreamOutcome::Done);
                        }
                        let value: serde_json::Value = match serde_json::from_str(&m.data) {
                            Ok(v) => v,
                            Err(e) => bail!("malformed stream chunk: {e}\nraw: {}", m.data),
                        };

                        // Extract delta text tokens
                        if let Some(choices) = value["choices"].as_array() {
                            for choice in choices {
                                if let Some(text) = choice["delta"]["content"].as_str() {
                                    if !text.is_empty() {
                                        if tx.send(StreamEvent::Token(text.to_string())).await.is_err() {
                                            es.close();
                                            return Ok(StreamOutcome::Cancelled);
                                        }
                                    }
                                }
                            }
                        }

                        // Extract usage from the final chunk
                        if let Some(usage) = value.get("usage") {
                            if let Some(total) = usage["total_tokens"].as_u64() {
                                let _ = tx.send(StreamEvent::Usage {
                                    total_tokens: total as usize,
                                }).await;
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

async fn format_error(cfg: &ResolvedConfig, err: reqwest_eventsource::Error) -> anyhow::Error {
    use reqwest_eventsource::Error as E;
    match err {
        E::InvalidStatusCode(status, resp) => {
            let body = resp.text().await.unwrap_or_default();
            anyhow::anyhow!(
                "request to {} failed with HTTP {}\nmodel: {}\nbody: {}",
                cfg.base_url,
                status,
                cfg.model,
                body
            )
        }
        E::Transport(e) => {
            anyhow::anyhow!(
                "transport error contacting {}: {}\nhint: check your network and that api.deepseek.com is reachable",
                cfg.base_url,
                e
            )
        }
        other => anyhow::anyhow!("stream error from {}: {}", cfg.base_url, other),
    }
}
