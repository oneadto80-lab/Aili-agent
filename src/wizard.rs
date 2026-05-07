use anyhow::{Context, Result, bail};
use std::io::{self, BufRead, Write};

use crate::config;
use crate::provider::Provider;

const HEADER: &str = "\n\
Welcome to Aili. Let's get you set up. (Ctrl-C anytime to abort)\n";

/// Run the first-run interactive wizard. Writes config + secrets to disk on
/// success.
pub fn run() -> Result<()> {
    println!("{HEADER}");

    let provider = ask_provider()?;
    let api_key = if provider.requires_api_key() {
        Some(ask_api_key(provider)?)
    } else {
        None
    };
    let model = ask_model(provider)?;
    let user_name = ask_user_name()?;

    config::write_wizard_result(provider, &model, &user_name, api_key.as_deref())?;

    println!();
    println!("✓ wrote {}", config::config_path()?.display());
    if let Some(k) = api_key.as_deref() {
        let secrets = config::secrets_path()?;
        if !secrets.exists() {
            bail!(
                "expected to have written {}, but it doesn't exist",
                secrets.display()
            );
        }
        println!("✓ wrote {} (chmod 600)", secrets.display());
        println!(
            "  key fingerprint: {}…{}  (length {})",
            &k.chars().take(4).collect::<String>(),
            &k.chars()
                .rev()
                .take(4)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>(),
            k.chars().count()
        );
    }
    println!();
    println!("Starting Aili...");
    println!();
    Ok(())
}

fn ask_provider() -> Result<Provider> {
    println!("[1/4] Which provider?");
    let providers = Provider::all();
    for (i, p) in providers.iter().enumerate() {
        let suffix = if matches!(p, Provider::DeepSeek) {
            "  (recommended)"
        } else {
            ""
        };
        println!("  {}) {}{}", i + 1, p.display_label(), suffix);
    }
    loop {
        let raw = prompt("> ")?;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            // Default to first option (DeepSeek).
            return Ok(providers[0]);
        }
        if let Ok(n) = trimmed.parse::<usize>() {
            if n >= 1 && n <= providers.len() {
                return Ok(providers[n - 1]);
            }
        }
        if let Ok(p) = Provider::parse(trimmed) {
            return Ok(p);
        }
        println!("  invalid choice; pick a number 1–{}.", providers.len());
    }
}

fn ask_api_key(provider: Provider) -> Result<String> {
    let env_name = provider.default_api_key_env().unwrap_or("PROVIDER_API_KEY");
    println!();
    println!("[2/4] Paste your {} API key.", provider.display_label());
    println!(
        "      Stored in {} (chmod 600).",
        config::secrets_path()?.display()
    );
    println!("      Or press Enter to skip and set the {env_name} env var manually later.");
    let key = rpassword::prompt_password("> ").context("failed to read API key from stdin")?;
    let key = key.trim().to_string();
    if key.is_empty() {
        bail!(
            "no API key provided; rerun the wizard or set {} manually",
            env_name
        );
    }
    Ok(key)
}

fn ask_model(provider: Provider) -> Result<String> {
    println!();
    println!("[3/4] Default model:");
    let presets = provider.model_presets();
    if presets.is_empty() {
        println!(
            "  no presets for {}. Type the model id you want as default.",
            provider.display_label()
        );
        loop {
            let raw = prompt("> ")?;
            let t = raw.trim().to_string();
            if !t.is_empty() {
                return Ok(t);
            }
            println!("  please enter a non-empty model id.");
        }
    }
    for (i, m) in presets.iter().enumerate() {
        let suffix = if i == 0 { "  (recommended)" } else { "" };
        println!("  {}) {}{}", i + 1, m, suffix);
    }
    println!("  (or type a custom model id)");
    loop {
        let raw = prompt("> ")?;
        let t = raw.trim();
        if t.is_empty() {
            return Ok(presets[0].to_string());
        }
        if let Ok(n) = t.parse::<usize>() {
            if n >= 1 && n <= presets.len() {
                return Ok(presets[n - 1].to_string());
            }
        }
        // Treat anything else as a custom model id.
        if !t.is_empty() {
            return Ok(t.to_string());
        }
    }
}

fn ask_user_name() -> Result<String> {
    println!();
    println!("[4/4] What should Aili call you?");
    loop {
        let raw = prompt("> ")?;
        let t = raw.trim().to_string();
        if t.is_empty() {
            // Empty -> "you" placeholder.
            return Ok("you".to_string());
        }
        if t.len() > 64 {
            println!("  please keep it under 64 characters.");
            continue;
        }
        return Ok(t);
    }
}

fn prompt(label: &str) -> Result<String> {
    print!("{label}");
    io::stdout().flush().ok();
    let mut buf = String::new();
    io::stdin()
        .lock()
        .read_line(&mut buf)
        .context("failed to read from stdin")?;
    if buf.is_empty() {
        // EOF reached.
        bail!("aborted (eof)");
    }
    Ok(buf)
}
