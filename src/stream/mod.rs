mod openai_compat;

use anyhow::Result;
use std::future::Future;
use tokio::sync::mpsc;

use crate::chat::Message;
use crate::config::ResolvedConfig;

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
    /// Usage info from the final stream chunk (total tokens used).
    Usage { total_tokens: usize },
}

/// Dispatch a streaming chat request to DeepSeek's OpenAI-compatible API.
pub async fn run_stream(
    client: &reqwest::Client,
    cfg: &ResolvedConfig,
    messages: &[Message],
    tx: mpsc::Sender<StreamEvent>,
    cancel: impl Future<Output = ()>,
) -> Result<StreamOutcome> {
    openai_compat::run(client, cfg, messages, tx, cancel).await
}
