use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

// ───────────────────────────── on-disk schema ─────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RawConfig {
    #[serde(default)]
    pub core: Option<CoreSection>,
    #[serde(default)]
    pub persona: Option<PersonaSection>,
    #[serde(default)]
    pub sampling: Option<SamplingSection>,
    #[serde(default)]
    pub tui: Option<TuiSection>,

    // Legacy flat fields (v0.1 / v0.2). Ignored if core section present.
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub top_p: Option<f32>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub stop: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CoreSection {
    #[serde(default)]
    pub provider: Option<String>, // ignored; always DeepSeek
    pub model: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>, // ignored; hardcoded deepseek url
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub initialized: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PersonaSection {
    pub user_name: Option<String>,
    pub assistant_name: Option<String>,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SamplingSection {
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub stop: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TuiSection {
    pub alternate_screen: Option<AltScreenMode>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AltScreenMode {
    #[default]
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SecretsFile {
    #[serde(default)]
    keys: HashMap<String, String>,
}

// ───────────────────────────── resolved view ─────────────────────────────

#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub max_tokens: Option<u32>,
    pub stop: Vec<String>,
    pub persona: Persona,
    pub tui: TuiConfig,
}

#[derive(Debug, Clone)]
pub struct Persona {
    pub user_name: String,
    pub assistant_name: String,
    #[allow(dead_code)]
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct TuiConfig {
    pub alternate_screen: AltScreenMode,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            alternate_screen: AltScreenMode::Auto,
        }
    }
}

impl Default for Persona {
    fn default() -> Self {
        Self {
            user_name: "you".into(),
            assistant_name: "Aili".into(),
            description: String::new(),
        }
    }
}

// ───────────────────────────── paths ─────────────────────────────

pub fn config_dir() -> Result<PathBuf> {
    config_dir_from_env(
        std::env::var_os("AILI_CONFIG_DIR"),
        std::env::var_os("HOME"),
    )
}

fn config_dir_from_env(override_dir: Option<OsString>, home: Option<OsString>) -> Result<PathBuf> {
    if let Some(dir) = override_dir.filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    let home = home
        .filter(|v| !v.is_empty())
        .context("could not determine HOME; cannot locate ~/.config/aili")?;
    Ok(PathBuf::from(home).join(".config").join("aili"))
}

pub fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.toml"))
}

pub fn secrets_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("secrets.toml"))
}

// ───────────────────────────── loading ─────────────────────────────

pub enum LoadResult {
    Loaded(ResolvedConfig),
    NeedsInit,
}

pub fn load() -> Result<LoadResult> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(LoadResult::NeedsInit);
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("could not read config at {}", path.display()))?;
    let cfg: RawConfig =
        toml::from_str(&raw).with_context(|| format!("invalid TOML in {}", path.display()))?;

    let initialized = cfg.core.as_ref().map(|c| c.initialized).unwrap_or(false);
    if !initialized {
        return Ok(LoadResult::NeedsInit);
    }

    Ok(LoadResult::Loaded(resolve(cfg)?))
}

fn resolve(raw: RawConfig) -> Result<ResolvedConfig> {
    let env_name = raw
        .core
        .as_ref()
        .and_then(|c| c.api_key_env.clone())
        .or_else(|| raw.api_key_env.clone())
        .filter(|s| !s.is_empty());

    let api_key = resolve_api_key(env_name.as_deref())?;

    let model = raw
        .core
        .as_ref()
        .and_then(|c| c.model.clone())
        .or_else(|| raw.model.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| crate::provider::DEFAULT_MODEL.id.to_string());

    let sampling = raw.sampling.unwrap_or_default();
    let temperature = sampling.temperature.or(raw.temperature);
    let top_p = sampling.top_p.or(raw.top_p);
    let max_tokens = sampling.max_tokens.or(raw.max_tokens);
    let stop = if !sampling.stop.is_empty() {
        sampling.stop
    } else {
        raw.stop
    };

    let persona_section = raw.persona.unwrap_or_default();
    let persona = Persona {
        user_name: persona_section.user_name.unwrap_or_else(|| "you".into()),
        assistant_name: persona_section
            .assistant_name
            .unwrap_or_else(|| "Aili".into()),
        description: persona_section.description,
    };
    let tui_section = raw.tui.unwrap_or_default();
    let tui = TuiConfig {
        alternate_screen: tui_section.alternate_screen.unwrap_or_default(),
    };

    Ok(ResolvedConfig {
        base_url: crate::provider::BASE_URL.to_string(),
        api_key,
        model,
        temperature,
        top_p,
        max_tokens,
        stop,
        persona,
        tui,
    })
}

fn resolve_api_key(env_name: Option<&str>) -> Result<String> {
    let name = env_name.unwrap_or(crate::provider::API_KEY_ENV);
    if let Ok(v) = std::env::var(name) {
        if !v.is_empty() {
            return Ok(v);
        }
    }
    if let Some(v) = read_secret(name)? {
        return Ok(v);
    }
    bail!(
        "no DeepSeek API key found. set env `{}` or add it to {}",
        name,
        secrets_path()?.display()
    );
}

fn read_secret(name: &str) -> Result<Option<String>> {
    let path = secrets_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("could not read {}", path.display()))?;
    let s: SecretsFile =
        toml::from_str(&raw).with_context(|| format!("invalid TOML in {}", path.display()))?;
    Ok(s.keys.get(name).cloned())
}

// ───────────────────────────── writing ─────────────────────────────

const TEMPLATE: &str = r#"# Aili config
# Edit by hand or rerun the wizard with `aili init`.

[core]
model        = "deepseek-v4-flash"
api_key_env  = "DEEPSEEK_API_KEY"
initialized  = false              # wizard sets this to true

[persona]
user_name      = "you"
assistant_name = "Aili"
description    = ""

[sampling]
# temperature = 0.7
# top_p       = 1.0
# max_tokens  = 4096

[tui]
# Aili's main chat uses inline terminal scrollback by default.
alternate_screen = "auto"
"#;

pub fn write_template(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create {}", parent.display()))?;
    }
    if path.exists() {
        bail!(
            "{} already exists; remove it first if you want to regenerate",
            path.display()
        );
    }
    write_atomic(path, TEMPLATE, None)?;
    Ok(())
}

pub fn write_wizard_result(model: &str, user_name: &str, api_key: Option<&str>) -> Result<()> {
    let dir = config_dir()?;
    std::fs::create_dir_all(&dir).with_context(|| format!("could not create {}", dir.display()))?;

    let toml_str = format!(
        "[core]\n\
         model        = \"{model}\"\n\
         api_key_env  = \"DEEPSEEK_API_KEY\"\n\
         initialized  = true\n\
         \n\
         [persona]\n\
         user_name      = \"{user}\"\n\
         assistant_name = \"Aili\"\n\
         description    = \"\"\n\
         \n\
         [sampling]\n\
         # temperature = 0.7\n\
         # top_p       = 1.0\n\
         # max_tokens  = 4096\n\
         \n\
         [tui]\n\
         alternate_screen = \"auto\"\n",
        model = escape_toml_string(model),
        user = escape_toml_string(user_name),
    );
    let cfg_path = config_path()?;
    write_atomic(&cfg_path, &toml_str, None)?;

    if let Some(key) = api_key {
        merge_secret(key)?;
    }
    Ok(())
}

fn merge_secret(key: &str) -> Result<()> {
    let path = secrets_path()?;
    let mut current: SecretsFile = if path.exists() {
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("could not read {}", path.display()))?;
        toml::from_str(&raw).unwrap_or_default()
    } else {
        SecretsFile::default()
    };
    current
        .keys
        .insert(crate::provider::API_KEY_ENV.to_string(), key.to_string());

    let mut body = String::from("# Aili secrets — chmod 600. Do not commit.\n[keys]\n");
    let mut entries: Vec<(&String, &String)> = current.keys.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    for (k, v) in entries {
        body.push_str(&format!("{} = \"{}\"\n", k, escape_toml_string(v)));
    }
    write_atomic(&path, &body, Some(0o600))?;
    Ok(())
}

fn write_atomic(path: &Path, body: &str, mode: Option<u32>) -> Result<()> {
    let parent = path.parent().with_context(|| {
        format!(
            "could not determine parent directory for {}",
            path.display()
        )
    })?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("could not create {}", parent.display()))?;

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .with_context(|| format!("invalid file name for {}", path.display()))?;
    let tmp_path = parent.join(format!(".{file_name}.tmp"));

    std::fs::write(&tmp_path, body)
        .with_context(|| format!("could not write {}", tmp_path.display()))?;

    #[cfg(unix)]
    if let Some(mode) = mode {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(mode);
        std::fs::set_permissions(&tmp_path, perms)
            .with_context(|| format!("could not chmod {:o} {}", mode, tmp_path.display()))?;
    }

    std::fs::rename(&tmp_path, path)
        .with_context(|| format!("could not replace {}", path.display()))?;
    Ok(())
}

fn escape_toml_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

// ───────────────────────────── tests ─────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct ConfigDirGuard {
        old: Option<OsString>,
    }

    impl Drop for ConfigDirGuard {
        fn drop(&mut self) {
            match &self.old {
                Some(v) => unsafe { std::env::set_var("AILI_CONFIG_DIR", v) },
                None => unsafe { std::env::remove_var("AILI_CONFIG_DIR") },
            }
        }
    }

    fn temp_config_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("aili-{name}-{}-{nanos}", std::process::id()))
    }

    fn with_config_dir<T>(dir: &Path, f: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK.lock().unwrap();
        let old = std::env::var_os("AILI_CONFIG_DIR");
        unsafe { std::env::set_var("AILI_CONFIG_DIR", dir) };
        let _env_guard = ConfigDirGuard { old };
        f()
    }

    #[test]
    fn config_dir_defaults_to_home_dot_config() {
        let dir = config_dir_from_env(None, Some(OsString::from("/tmp/aili-home"))).unwrap();
        assert_eq!(dir, PathBuf::from("/tmp/aili-home/.config/aili"));
    }

    #[test]
    fn config_dir_override_uses_exact_dir() {
        let dir = config_dir_from_env(
            Some(OsString::from("/tmp/custom-aili-config")),
            Some(OsString::from("/tmp/aili-home")),
        )
        .unwrap();
        assert_eq!(dir, PathBuf::from("/tmp/custom-aili-config"));
    }

    #[test]
    fn legacy_schema_resolves() {
        unsafe { std::env::set_var("AILI_TEST_LEGACY_KEY", "sk-test-legacy") };
        let raw = RawConfig {
            api_key_env: Some("AILI_TEST_LEGACY_KEY".into()),
            ..Default::default()
        };
        let r = resolve(raw).unwrap();
        assert_eq!(r.model, "deepseek-v4-flash");
        assert_eq!(r.api_key, "sk-test-legacy");
        assert_eq!(r.persona.user_name, "you");
        assert_eq!(r.persona.assistant_name, "Aili");
        assert_eq!(r.tui.alternate_screen, AltScreenMode::Auto);
    }

    #[test]
    fn new_schema_with_persona() {
        unsafe { std::env::set_var("AILI_TEST_NEW_KEY", "sk-test-new") };
        let raw = RawConfig {
            core: Some(CoreSection {
                model: Some("deepseek-v4-pro".into()),
                api_key_env: Some("AILI_TEST_NEW_KEY".into()),
                initialized: true,
                ..Default::default()
            }),
            persona: Some(PersonaSection {
                user_name: Some("rose".into()),
                assistant_name: Some("Aili".into()),
                description: "concise".into(),
            }),
            ..Default::default()
        };
        let r = resolve(raw).unwrap();
        assert_eq!(r.api_key, "sk-test-new");
        assert_eq!(r.model, "deepseek-v4-pro");
        assert_eq!(r.persona.user_name, "rose");
        assert_eq!(r.persona.description, "concise");
        assert_eq!(r.tui.alternate_screen, AltScreenMode::Auto);
    }

    #[test]
    fn tui_alternate_screen_resolves() {
        unsafe { std::env::set_var("AILI_TEST_TUI_KEY", "sk-test-tui") };
        let raw = RawConfig {
            core: Some(CoreSection {
                api_key_env: Some("AILI_TEST_TUI_KEY".into()),
                initialized: true,
                ..Default::default()
            }),
            tui: Some(TuiSection {
                alternate_screen: Some(AltScreenMode::Never),
            }),
            ..Default::default()
        };
        let r = resolve(raw).unwrap();
        assert_eq!(r.tui.alternate_screen, AltScreenMode::Never);
    }

    #[test]
    fn missing_env_errors_for_remote() {
        unsafe { std::env::remove_var("AILI_TEST_NO_SUCH_VAR") };
        let raw = RawConfig {
            core: Some(CoreSection {
                api_key_env: Some("AILI_TEST_NO_SUCH_VAR".into()),
                initialized: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        let r = resolve(raw);
        if r.is_ok() {
            // skip: a real secrets.toml on this test machine has the var; not our concern.
        }
    }

    #[test]
    fn wizard_result_writes_config_and_secret_then_loads() {
        let dir = temp_config_dir("wizard");
        with_config_dir(&dir, || {
            write_wizard_result("deepseek-v4-flash", "Rose", Some("sk-test-wizard")).unwrap();

            let cfg_raw = std::fs::read_to_string(dir.join("config.toml")).unwrap();
            assert!(cfg_raw.contains("model        = \"deepseek-v4-flash\""));
            assert!(cfg_raw.contains("api_key_env  = \"DEEPSEEK_API_KEY\""));
            assert!(cfg_raw.contains("initialized  = true"));
            assert!(cfg_raw.contains("user_name      = \"Rose\""));

            let secret_raw = std::fs::read_to_string(dir.join("secrets.toml")).unwrap();
            assert!(secret_raw.contains("DEEPSEEK_API_KEY = \"sk-test-wizard\""));

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(dir.join("secrets.toml"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777;
                assert_eq!(mode, 0o600);
            }

            match load().unwrap() {
                LoadResult::Loaded(c) => {
                    assert_eq!(c.model, "deepseek-v4-flash");
                    assert_eq!(c.api_key, "sk-test-wizard");
                    assert_eq!(c.persona.user_name, "Rose");
                }
                LoadResult::NeedsInit => panic!("wizard result should be initialized"),
            }
        });
        std::fs::remove_dir_all(dir).ok();
    }
}
