use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use reqwest_eventsource::{Event, EventSource};
use std::future::Future;
use tokio::sync::mpsc;

use crate::chat::{ChatRequest, Message, StreamChunk};
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
                        let chunk: StreamChunk = match serde_json::from_str(&m.data) {
                            Ok(c) => c,
                            Err(e) => bail!("malformed stream chunk: {e}\nraw: {}", m.data),
                        };
                        for choice in &chunk.choices {
                            if let Some(text) = &choice.delta.content {
                                if tx.send(StreamEvent::Token(text.clone())).await.is_err() {
                                    es.close();
                                    return Ok(StreamOutcome::Cancelled);
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

async fn format_error(cfg: &ResolvedConfig, err: reqwest_eventsource::Error) -> anyhow::Error {
    use reqwest_eventsource::Error as E;
    match err {
        E::InvalidStatusCode(status, resp) => {
            let body = resp.text().await.unwrap_or_default();
            anyhow::anyhow!(
                "request to {} failed with HTTP {}\nprovider: {}  model: {}\nbody: {}",
                cfg.base_url,
                status,
                cfg.provider.as_str(),
                cfg.model,
                body
            )
        }
        E::Transport(e) => {
            let mut msg = format!(
                "transport error contacting {} ({}): {}",
                cfg.base_url,
                cfg.provider.as_str(),
                e
            );
            if cfg.provider.is_local() && (e.is_connect() || e.is_request()) {
                msg.push_str("\nhint: ");
                msg.push_str(cfg.provider.unreachable_hint());
            }
            anyhow::anyhow!(msg)
        }
        other => anyhow::anyhow!(
            "stream error from {} ({}): {}",
            cfg.base_url,
            cfg.provider.as_str(),
            other
        ),
    }
}
