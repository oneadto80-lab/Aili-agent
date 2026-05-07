use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::provider::Provider;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawConfig {
    pub provider: String,
    pub base_url: Option<String>,
    pub api_key_env: Option<String>,
    pub model: Option<String>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub stop: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub provider: Provider,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub max_tokens: Option<u32>,
    pub stop: Vec<String>,
}

pub fn config_path() -> Result<PathBuf> {
    let dir = dirs::config_dir().context("could not determine user config dir")?;
    Ok(dir.join("aili").join("config.toml"))
}

pub fn load() -> Result<ResolvedConfig> {
    let path = config_path()?;
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("could not read config at {}. run `aili config init` first", path.display()))?;
    let cfg: RawConfig = toml::from_str(&raw)
        .with_context(|| format!("invalid TOML in {}", path.display()))?;
    resolve(cfg)
}

pub fn resolve(raw: RawConfig) -> Result<ResolvedConfig> {
    let provider = Provider::parse(&raw.provider)?;
    let base_url = raw
        .base_url
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| provider.default_base_url().to_string())
        .trim_end_matches('/')
        .to_string();
    let env_name = raw
        .api_key_env
        .clone()
        .or_else(|| provider.default_api_key_env().map(|s| s.to_string()));
    let api_key = provider.resolve_api_key(env_name.as_deref())?;
    let model = raw
        .model
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| provider.default_model().to_string());
    Ok(ResolvedConfig {
        provider,
        base_url,
        api_key,
        model,
        temperature: raw.temperature,
        top_p: raw.top_p,
        max_tokens: raw.max_tokens,
        stop: raw.stop,
    })
}

const TEMPLATE: &str = r#"# Aili config
# Pick a provider and (optionally) override defaults.

provider = "deepseek"            # deepseek | ollama | lmstudio | vllm

# Override only if you need to. Leave commented to use provider defaults.
# base_url    = "https://api.deepseek.com/v1"
# api_key_env = "DEEPSEEK_API_KEY"   # name of env var, NOT the key itself
# model       = "deepseek-v4-flash"  # deepseek-v4-flash | deepseek-v4-pro | deepseek-chat | deepseek-reasoner

# Sampling parameters (all optional).
# temperature = 0.7
# top_p       = 1.0
# max_tokens  = 4096
# stop        = []
"#;

pub fn write_template(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create {}", parent.display()))?;
    }
    if path.exists() {
        anyhow::bail!("{} already exists; remove it first if you want to regenerate", path.display());
    }
    std::fs::write(path, TEMPLATE)
        .with_context(|| format!("could not write {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deepseek_defaults_apply() {
        unsafe { std::env::set_var("DEEPSEEK_API_KEY", "sk-test") };
        let raw = RawConfig {
            provider: "deepseek".into(),
            base_url: None,
            api_key_env: None,
            model: None,
            temperature: None,
            top_p: None,
            max_tokens: None,
            stop: vec![],
        };
        let r = resolve(raw).unwrap();
        assert_eq!(r.base_url, "https://api.deepseek.com/v1");
        assert_eq!(r.model, "deepseek-v4-flash");
        assert_eq!(r.api_key, "sk-test");
    }

    #[test]
    fn ollama_needs_no_key() {
        let raw = RawConfig {
            provider: "ollama".into(),
            base_url: None,
            api_key_env: None,
            model: Some("qwen2.5".into()),
            temperature: None,
            top_p: None,
            max_tokens: None,
            stop: vec![],
        };
        let r = resolve(raw).unwrap();
        assert_eq!(r.base_url, "http://localhost:11434/v1");
        assert_eq!(r.api_key, "ollama");
        assert_eq!(r.model, "qwen2.5");
    }

    #[test]
    fn deepseek_missing_env_errors() {
        unsafe { std::env::remove_var("AILI_TEST_NO_SUCH_VAR") };
        let raw = RawConfig {
            provider: "deepseek".into(),
            base_url: None,
            api_key_env: Some("AILI_TEST_NO_SUCH_VAR".into()),
            model: None,
            temperature: None,
            top_p: None,
            max_tokens: None,
            stop: vec![],
        };
        assert!(resolve(raw).is_err());
    }

    #[test]
    fn trailing_slash_trimmed() {
        let raw = RawConfig {
            provider: "ollama".into(),
            base_url: Some("http://localhost:11434/v1/".into()),
            api_key_env: None,
            model: None,
            temperature: None,
            top_p: None,
            max_tokens: None,
            stop: vec![],
        };
        let r = resolve(raw).unwrap();
        assert_eq!(r.base_url, "http://localhost:11434/v1");
    }
}
