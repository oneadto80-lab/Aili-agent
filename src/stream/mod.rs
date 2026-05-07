mod anthropic;
mod gemini;
mod openai_compat;

use anyhow::Result;
use std::future::Future;
use tokio::sync::mpsc;

use crate::chat::Message;
use crate::config::ResolvedConfig;
use crate::provider::Protocol;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamOutcome {
    /// Server signalled end of stream cleanly.
    Done,
    /// User cancelled mid-stream.
    Cancelled,
}

#[derive(Debug, Clone)]
pub enum StreamEvent {
    Token(String),
}

/// Dispatch a streaming chat request to the right protocol adapter. Tokens
/// arrive through `tx`; cancellation is signalled by `cancel` resolving.
pub async fn run_stream(
    client: &reqwest::Client,
    cfg: &ResolvedConfig,
    messages: &[Message],
    tx: mpsc::Sender<StreamEvent>,
    cancel: impl Future<Output = ()>,
) -> Result<StreamOutcome> {
    match cfg.provider.protocol() {
        Protocol::OpenAICompat => openai_compat::run(client, cfg, messages, tx, cancel).await,
        Protocol::Anthropic => anthropic::run(client, cfg, messages, tx, cancel).await,
        Protocol::Gemini => gemini::run(client, cfg, messages, tx, cancel).await,
    }
}

/// For local providers, do a cheap GET /models to surface "server is down"
/// before we dive into a streaming POST. Skipped for remote providers.
pub async fn probe_local(client: &reqwest::Client, cfg: &ResolvedConfig) -> Result<()> {
    if !cfg.provider.is_local() {
        return Ok(());
    }
    let url = format!("{}/models", cfg.base_url);
    match client.get(&url).bearer_auth(&cfg.api_key).send().await {
        Ok(_) => Ok(()),
        Err(e) => anyhow::bail!(
            "could not reach {} ({}): {}\nhint: {}",
            cfg.base_url,
            cfg.provider.as_str(),
            e,
            cfg.provider.unreachable_hint()
        ),
    }
}
