use serde::{Deserialize, Serialize};

use crate::config::ResolvedConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

impl Message {
    pub fn system(s: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: s.into(),
        }
    }
    pub fn user(s: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: s.into(),
        }
    }
    pub fn assistant(s: impl Into<String>) -> Self {
        Self {
            role: "assistant".into(),
            content: s.into(),
        }
    }
}

pub fn persona_system_prompt(cfg: &ResolvedConfig) -> String {
    let mut prompt = format!(
        "You are Aili, a local-first terminal companion agent.\n\
         The user's preferred name is \"{}\".\n\
         Always identify yourself as Aili. Do not identify yourself as {}, {}, DeepSeek, OpenAI, Anthropic, Gemini, Google, or any backend model/provider.\n\
         The provider and model are implementation details only.\n\
         Reply in the user's language. Be concise, direct, and useful.",
        cfg.persona.user_name,
        cfg.provider.as_str(),
        cfg.model,
    );
    if !cfg.persona.description.trim().is_empty() {
        prompt.push_str("\nAdditional persona notes: ");
        prompt.push_str(cfg.persona.description.trim());
    }
    prompt
}

pub fn prepend_system_prompt(cfg: &ResolvedConfig, mut messages: Vec<Message>) -> Vec<Message> {
    let mut out = Vec::with_capacity(messages.len() + 1);
    out.push(Message::system(persona_system_prompt(cfg)));
    out.append(&mut messages);
    out
}

#[derive(Debug, Serialize)]
pub struct ChatRequest<'a> {
    pub model: &'a str,
    pub messages: &'a [Message],
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub stop: Vec<String>,
}

impl<'a> ChatRequest<'a> {
    pub fn build(cfg: &'a ResolvedConfig, messages: &'a [Message]) -> Self {
        Self {
            model: &cfg.model,
            messages,
            stream: true,
            temperature: cfg.temperature,
            top_p: cfg.top_p,
            max_tokens: cfg.max_tokens,
            stop: cfg.stop.clone(),
        }
    }
}

/// Streaming SSE chunk shape (OpenAI-compatible).
#[derive(Debug, Deserialize)]
pub struct StreamChunk {
    #[serde(default)]
    pub choices: Vec<StreamChoice>,
}

#[derive(Debug, Deserialize)]
pub struct StreamChoice {
    #[serde(default)]
    pub delta: StreamDelta,
    #[serde(default)]
    #[allow(dead_code)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct StreamDelta {
    #[serde(default)]
    pub content: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Persona;
    use crate::provider::Provider;

    fn cfg() -> ResolvedConfig {
        ResolvedConfig {
            provider: Provider::DeepSeek,
            base_url: "https://api.deepseek.com/v1".into(),
            api_key: "sk-x".into(),
            model: "deepseek-v4-flash".into(),
            temperature: Some(0.5),
            top_p: None,
            max_tokens: Some(128),
            stop: vec![],
            persona: Persona::default(),
        }
    }

    #[test]
    fn request_serializes_messages_and_stream() {
        let c = cfg();
        let msgs = prepend_system_prompt(&c, vec![Message::user("hi")]);
        let req = ChatRequest::build(&c, &msgs);
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["model"], "deepseek-v4-flash");
        assert_eq!(v["stream"], true);
        assert_eq!(v["messages"][0]["role"], "system");
        assert!(
            v["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("Aili")
        );
        assert!(
            v["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("Do not identify yourself")
        );
        assert_eq!(v["messages"][1]["role"], "user");
        assert_eq!(v["messages"][1]["content"], "hi");
        assert_eq!(v["temperature"], 0.5);
        assert_eq!(v["max_tokens"], 128);
        assert!(v.get("top_p").is_none());
        assert!(v.get("stop").is_none());
    }

    #[test]
    fn persona_prompt_contains_user_and_identity_guardrails() {
        let mut c = cfg();
        c.persona.user_name = "Rose".into();
        let prompt = persona_system_prompt(&c);
        assert!(prompt.contains("Aili"));
        assert!(prompt.contains("Rose"));
        assert!(prompt.contains("Do not identify yourself"));
        assert!(prompt.contains("deepseek"));
        assert!(prompt.contains("deepseek-v4-flash"));
    }

    #[test]
    fn parses_delta_content() {
        let json = r#"{"choices":[{"delta":{"content":"hello"},"finish_reason":null}]}"#;
        let c: StreamChunk = serde_json::from_str(json).unwrap();
        assert_eq!(c.choices[0].delta.content.as_deref(), Some("hello"));
    }

    #[test]
    fn parses_empty_delta() {
        let json = r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#;
        let c: StreamChunk = serde_json::from_str(json).unwrap();
        assert!(c.choices[0].delta.content.is_none());
        assert_eq!(c.choices[0].finish_reason.as_deref(), Some("stop"));
    }
}
