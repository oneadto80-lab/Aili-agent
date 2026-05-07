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
struct GeminiRequest {
    contents: Vec<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "systemInstruction")]
    system_instruction: Option<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "generationConfig")]
    generation_config: Option<GenerationConfig>,
}

#[derive(Debug, Serialize)]
struct GeminiContent {
    role: String,
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Serialize)]
struct GeminiPart {
    text: String,
}

#[derive(Debug, Serialize, Default)]
struct GenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "topP")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "maxOutputTokens")]
    max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Vec::is_empty", rename = "stopSequences")]
    stop_sequences: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct GeminiChunk {
    #[serde(default)]
    candidates: Vec<GeminiCandidate>,
}

#[derive(Debug, Deserialize)]
struct GeminiCandidate {
    #[serde(default)]
    content: Option<GeminiContentResp>,
    #[serde(default, rename = "finishReason")]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GeminiContentResp {
    #[serde(default)]
    parts: Vec<GeminiPartResp>,
}

#[derive(Debug, Deserialize)]
struct GeminiPartResp {
    #[serde(default)]
    text: Option<String>,
}

/// POST {base_url}/models/{model}:streamGenerateContent?alt=sse and parse the
/// response as SSE-formatted JSON chunks.
pub async fn run(
    client: &reqwest::Client,
    cfg: &ResolvedConfig,
    messages: &[Message],
    tx: mpsc::Sender<StreamEvent>,
    cancel: impl Future<Output = ()>,
) -> Result<StreamOutcome> {
    let url = format!(
        "{}/models/{}:streamGenerateContent?alt=sse",
        cfg.base_url, cfg.model
    );

    let (system, conv) = split_system(messages);
    let body = GeminiRequest {
        contents: conv
            .into_iter()
            .map(|m| GeminiContent {
                role: gemini_role(&m.role),
                parts: vec![GeminiPart {
                    text: m.content.clone(),
                }],
            })
            .collect(),
        system_instruction: system.map(|t| GeminiContent {
            role: "system".to_string(),
            parts: vec![GeminiPart { text: t }],
        }),
        generation_config: Some(GenerationConfig {
            temperature: cfg.temperature,
            top_p: cfg.top_p,
            max_output_tokens: cfg.max_tokens,
            stop_sequences: cfg.stop.clone(),
        }),
    };

    let req = client
        .post(&url)
        .header("x-goog-api-key", &cfg.api_key)
        .header("content-type", "application/json")
        .json(&body);

    let mut es = EventSource::new(req).context("failed to start Gemini SSE request")?;
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
                        let chunk: GeminiChunk = match serde_json::from_str(&m.data) {
                            Ok(c) => c,
                            Err(_) => continue,
                        };
                        let mut finished = false;
                        for cand in chunk.candidates {
                            if let Some(c) = cand.content {
                                for p in c.parts {
                                    if let Some(text) = p.text {
                                        if !text.is_empty()
                                            && tx.send(StreamEvent::Token(text)).await.is_err()
                                        {
                                            es.close();
                                            return Ok(StreamOutcome::Cancelled);
                                        }
                                    }
                                }
                            }
                            if cand.finish_reason.is_some() {
                                finished = true;
                            }
                        }
                        if finished {
                            es.close();
                            return Ok(StreamOutcome::Done);
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

fn gemini_role(role: &str) -> String {
    // Gemini calls the assistant "model" rather than "assistant".
    match role {
        "assistant" => "model".to_string(),
        other => other.to_string(),
    }
}

async fn format_error(cfg: &ResolvedConfig, err: reqwest_eventsource::Error) -> anyhow::Error {
    use reqwest_eventsource::Error as E;
    match err {
        E::InvalidStatusCode(status, resp) => {
            let body = resp.text().await.unwrap_or_default();
            anyhow::anyhow!(
                "Gemini request to {} failed with HTTP {}\nmodel: {}\nbody: {}",
                cfg.base_url,
                status,
                cfg.model,
                body
            )
        }
        E::Transport(e) => anyhow::anyhow!(
            "transport error contacting {} (gemini): {}",
            cfg.base_url,
            e
        ),
        other => anyhow::anyhow!("Gemini stream error: {}", other),
    }
}
