use anyhow::{Result, bail};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    DeepSeek,
    OpenAI,
    Anthropic,
    Gemini,
    Ollama,
    LmStudio,
    Vllm,
}

/// API protocol family. Maps a provider to one of three known wire shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// OpenAI-compatible /v1/chat/completions streaming. Used by OpenAI itself,
    /// DeepSeek, Ollama, LM Studio, vLLM.
    OpenAICompat,
    /// Anthropic /v1/messages streaming.
    Anthropic,
    /// Google Gemini :streamGenerateContent (SSE).
    Gemini,
}

impl Provider {
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s.to_ascii_lowercase().as_str() {
            "deepseek" => Provider::DeepSeek,
            "openai" => Provider::OpenAI,
            "anthropic" => Provider::Anthropic,
            "gemini" | "google" => Provider::Gemini,
            "ollama" => Provider::Ollama,
            "lmstudio" | "lm-studio" | "lm_studio" => Provider::LmStudio,
            "vllm" => Provider::Vllm,
            other => bail!(
                "unknown provider: {other}. expected one of: deepseek, openai, anthropic, gemini, ollama, lmstudio, vllm"
            ),
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Provider::DeepSeek => "deepseek",
            Provider::OpenAI => "openai",
            Provider::Anthropic => "anthropic",
            Provider::Gemini => "gemini",
            Provider::Ollama => "ollama",
            Provider::LmStudio => "lmstudio",
            Provider::Vllm => "vllm",
        }
    }

    pub fn protocol(self) -> Protocol {
        match self {
            Provider::Anthropic => Protocol::Anthropic,
            Provider::Gemini => Protocol::Gemini,
            _ => Protocol::OpenAICompat,
        }
    }

    pub fn default_base_url(self) -> &'static str {
        match self {
            Provider::DeepSeek => "https://api.deepseek.com/v1",
            Provider::OpenAI => "https://api.openai.com/v1",
            Provider::Anthropic => "https://api.anthropic.com/v1",
            Provider::Gemini => "https://generativelanguage.googleapis.com/v1beta",
            Provider::Ollama => "http://localhost:11434/v1",
            Provider::LmStudio => "http://localhost:1234/v1",
            Provider::Vllm => "http://localhost:8000/v1",
        }
    }

    pub fn default_api_key_env(self) -> Option<&'static str> {
        match self {
            Provider::DeepSeek => Some("DEEPSEEK_API_KEY"),
            Provider::OpenAI => Some("OPENAI_API_KEY"),
            Provider::Anthropic => Some("ANTHROPIC_API_KEY"),
            Provider::Gemini => Some("GOOGLE_API_KEY"),
            Provider::Ollama | Provider::LmStudio | Provider::Vllm => None,
        }
    }

    pub fn default_model(self) -> &'static str {
        match self {
            Provider::DeepSeek => "deepseek-v4-flash",
            Provider::OpenAI => "gpt-5.5",
            Provider::Anthropic => "claude-opus-4-7",
            Provider::Gemini => "gemini-2.5-pro",
            Provider::Ollama => "llama3.2",
            Provider::LmStudio => "local-model",
            Provider::Vllm => "local-model",
        }
    }

    /// Recommended model presets shown by the wizard. The first entry is the
    /// recommended default.
    pub fn model_presets(self) -> &'static [&'static str] {
        match self {
            Provider::DeepSeek => &["deepseek-v4-flash", "deepseek-v4-pro"],
            Provider::OpenAI => &["gpt-5.5"],
            Provider::Anthropic => &["claude-opus-4-7"],
            Provider::Gemini => &["gemini-2.5-pro"],
            Provider::Ollama | Provider::LmStudio | Provider::Vllm => &[],
        }
    }

    pub fn is_local(self) -> bool {
        matches!(self, Provider::Ollama | Provider::LmStudio | Provider::Vllm)
    }

    pub fn requires_api_key(self) -> bool {
        !self.is_local()
    }

    pub fn unreachable_hint(self) -> &'static str {
        match self {
            Provider::Ollama => {
                "is Ollama running? try `ollama serve` and verify the model is pulled (`ollama pull <model>`)"
            }
            Provider::LmStudio => {
                "is LM Studio's local server started? open the app, go to Local Server, and click Start"
            }
            Provider::Vllm => {
                "is vLLM running? check `vllm serve <model>`, the port, and the API key"
            }
            Provider::DeepSeek => "check your network and that api.deepseek.com is reachable",
            Provider::OpenAI => "check your network and that api.openai.com is reachable",
            Provider::Anthropic => "check your network and that api.anthropic.com is reachable",
            Provider::Gemini => {
                "check your network and that generativelanguage.googleapis.com is reachable"
            }
        }
    }

    /// Display label for the wizard menu.
    pub fn display_label(self) -> &'static str {
        match self {
            Provider::DeepSeek => "DeepSeek",
            Provider::OpenAI => "OpenAI",
            Provider::Anthropic => "Anthropic",
            Provider::Gemini => "Google Gemini",
            Provider::Ollama => "Ollama (local)",
            Provider::LmStudio => "LM Studio (local)",
            Provider::Vllm => "vLLM (local)",
        }
    }

    /// All providers in canonical menu order.
    pub fn all() -> &'static [Provider] {
        &[
            Provider::DeepSeek,
            Provider::OpenAI,
            Provider::Anthropic,
            Provider::Gemini,
            Provider::Ollama,
            Provider::LmStudio,
            Provider::Vllm,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_dispatch() {
        assert_eq!(Provider::DeepSeek.protocol(), Protocol::OpenAICompat);
        assert_eq!(Provider::OpenAI.protocol(), Protocol::OpenAICompat);
        assert_eq!(Provider::Ollama.protocol(), Protocol::OpenAICompat);
        assert_eq!(Provider::Anthropic.protocol(), Protocol::Anthropic);
        assert_eq!(Provider::Gemini.protocol(), Protocol::Gemini);
    }

    #[test]
    fn parse_aliases() {
        assert_eq!(Provider::parse("openai").unwrap(), Provider::OpenAI);
        assert_eq!(Provider::parse("Google").unwrap(), Provider::Gemini);
        assert_eq!(Provider::parse("lm-studio").unwrap(), Provider::LmStudio);
    }
}
