use anyhow::{Context, Result, bail};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    DeepSeek,
    Ollama,
    LmStudio,
    Vllm,
}

impl Provider {
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s.to_ascii_lowercase().as_str() {
            "deepseek" => Provider::DeepSeek,
            "ollama" => Provider::Ollama,
            "lmstudio" | "lm-studio" | "lm_studio" => Provider::LmStudio,
            "vllm" => Provider::Vllm,
            other => bail!("unknown provider: {other}. expected one of: deepseek, ollama, lmstudio, vllm"),
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Provider::DeepSeek => "deepseek",
            Provider::Ollama => "ollama",
            Provider::LmStudio => "lmstudio",
            Provider::Vllm => "vllm",
        }
    }

    pub fn default_base_url(self) -> &'static str {
        match self {
            Provider::DeepSeek => "https://api.deepseek.com/v1",
            Provider::Ollama => "http://localhost:11434/v1",
            Provider::LmStudio => "http://localhost:1234/v1",
            Provider::Vllm => "http://localhost:8000/v1",
        }
    }

    pub fn default_api_key_env(self) -> Option<&'static str> {
        match self {
            Provider::DeepSeek => Some("DEEPSEEK_API_KEY"),
            Provider::Ollama => None,
            Provider::LmStudio => None,
            Provider::Vllm => None,
        }
    }

    pub fn default_model(self) -> &'static str {
        match self {
            Provider::DeepSeek => "deepseek-v4-flash",
            Provider::Ollama => "llama3.2",
            Provider::LmStudio => "local-model",
            Provider::Vllm => "local-model",
        }
    }

    pub fn is_local(self) -> bool {
        matches!(self, Provider::Ollama | Provider::LmStudio | Provider::Vllm)
    }

    /// Hint to show when the local server appears unreachable.
    pub fn unreachable_hint(self) -> &'static str {
        match self {
            Provider::Ollama => "is Ollama running? try `ollama serve` and verify the model is pulled (`ollama pull <model>`)",
            Provider::LmStudio => "is LM Studio's local server started? open the app, go to Local Server, and click Start",
            Provider::Vllm => "is vLLM running? check `vllm serve <model>`, the port, and the API key",
            Provider::DeepSeek => "check your network and that api.deepseek.com is reachable",
        }
    }

    /// Effective API key: read from env, or fall back to a placeholder for
    /// providers that don't authenticate (Ollama).
    pub fn resolve_api_key(self, env_name: Option<&str>) -> Result<String> {
        if let Some(name) = env_name.filter(|s| !s.is_empty()) {
            return std::env::var(name)
                .with_context(|| format!("env var `{name}` not set (required for provider {})", self.as_str()));
        }
        match self {
            Provider::Ollama => Ok("ollama".to_string()),
            Provider::LmStudio | Provider::Vllm => Ok("local".to_string()),
            Provider::DeepSeek => bail!("DeepSeek requires an api_key_env (default: DEEPSEEK_API_KEY)"),
        }
    }
}
