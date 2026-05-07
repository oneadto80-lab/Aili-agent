use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use reqwest_eventsource::{Event, EventSource};
use std::future::Future;
use tokio::io::{AsyncWriteExt, stdout};

use crate::chat::{ChatRequest, Message, StreamChunk};
use crate::config::ResolvedConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamOutcome {
    /// Server sent its terminal `[DONE]` (or the stream closed cleanly).
    Done,
    /// User cancelled mid-stream.
    Cancelled,
}

/// POST a streaming chat completion. Writes raw token deltas to stdout as they
/// arrive, and appends the full text into `sink` so the caller can keep
/// conversation context in REPL mode.
///
/// `cancel` is awaited concurrently with the event stream; if it resolves
/// first, we abort and return `Cancelled`.
pub async fn run_stream(
    client: &reqwest::Client,
    cfg: &ResolvedConfig,
    messages: &[Message],
    sink: &mut String,
    cancel: impl Future<Output = ()>,
) -> Result<StreamOutcome> {
    let url = format!("{}/chat/completions", cfg.base_url);
    let body = ChatRequest::build(cfg, messages);

    let req = client
        .post(&url)
        .bearer_auth(&cfg.api_key)
        .json(&body);

    let mut es = EventSource::new(req).context("failed to start SSE request")?;
    tokio::pin!(cancel);

    let mut out = stdout();

    loop {
        tokio::select! {
            biased;
            _ = &mut cancel => {
                es.close();
                out.write_all(b"\n").await.ok();
                out.flush().await.ok();
                return Ok(StreamOutcome::Cancelled);
            }
            ev = es.next() => {
                let Some(ev) = ev else {
                    out.write_all(b"\n").await.ok();
                    out.flush().await.ok();
                    return Ok(StreamOutcome::Done);
                };
                match ev {
                    Ok(Event::Open) => {}
                    Ok(Event::Message(m)) => {
                        if m.data.trim() == "[DONE]" {
                            es.close();
                            out.write_all(b"\n").await.ok();
                            out.flush().await.ok();
                            return Ok(StreamOutcome::Done);
                        }
                        let chunk: StreamChunk = match serde_json::from_str(&m.data) {
                            Ok(c) => c,
                            Err(e) => {
                                bail!("malformed stream chunk: {e}\nraw: {}", m.data);
                            }
                        };
                        for choice in &chunk.choices {
                            if let Some(text) = &choice.delta.content {
                                sink.push_str(text);
                                out.write_all(text.as_bytes()).await.ok();
                                out.flush().await.ok();
                            }
                        }
                    }
                    Err(reqwest_eventsource::Error::StreamEnded) => {
                        out.write_all(b"\n").await.ok();
                        out.flush().await.ok();
                        return Ok(StreamOutcome::Done);
                    }
                    Err(e) => {
                        es.close();
                        return Err(format_stream_error(cfg, e).await);
                    }
                }
            }
        }
    }
}

async fn format_stream_error(cfg: &ResolvedConfig, err: reqwest_eventsource::Error) -> anyhow::Error {
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

/// For local providers, do a cheap GET /models to surface "server is down"
/// before we dive into a streaming POST.
pub async fn probe_local(client: &reqwest::Client, cfg: &ResolvedConfig) -> Result<()> {
    if !cfg.provider.is_local() {
        return Ok(());
    }
    let url = format!("{}/models", cfg.base_url);
    match client.get(&url).bearer_auth(&cfg.api_key).send().await {
        Ok(_) => Ok(()),
        Err(e) => {
            anyhow::bail!(
                "could not reach {} ({}): {}\nhint: {}",
                cfg.base_url,
                cfg.provider.as_str(),
                e,
                cfg.provider.unreachable_hint()
            )
        }
    }
}
