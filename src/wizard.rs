use anyhow::{Context, Result, bail};
use std::io::{self, BufRead, Write};

use crate::config;
use crate::provider::{self, DeepSeekModel, V4_FLASH, V4_PRO};

const HEADER: &str = "\n\
Welcome to Aili. Let's get you set up. (Ctrl-C anytime to abort)\n";

pub fn run() -> Result<()> {
    println!("{HEADER}");

    let api_key = ask_api_key()?;
    let model = ask_model()?;
    let user_name = ask_user_name()?;

    config::write_wizard_result(&model.id, &user_name, Some(&api_key))?;

    println!();
    println!("✓ wrote {}", config::config_path()?.display());
    let secrets = config::secrets_path()?;
    println!("✓ wrote {} (chmod 600)", secrets.display());
    println!(
        "  key fingerprint: {}…{}  (length {})",
        &api_key.chars().take(4).collect::<String>(),
        &api_key
            .chars()
            .rev()
            .take(4)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>(),
        api_key.chars().count()
    );
    println!();
    println!("Starting Aili...");
    println!();
    Ok(())
}

fn ask_api_key() -> Result<String> {
    println!();
    println!("[1/3] Paste your DeepSeek API key.");
    println!(
        "      Stored in {} (chmod 600).",
        config::secrets_path()?.display()
    );
    println!("      Or press Enter to skip and set the DEEPSEEK_API_KEY env var manually later.");
    let key = rpassword::prompt_password("> ").context("failed to read API key from stdin")?;
    let key = key.trim().to_string();
    if key.is_empty() {
        bail!(
            "no API key provided; rerun the wizard or set {} manually",
            provider::API_KEY_ENV
        );
    }
    Ok(key)
}

fn ask_model() -> Result<&'static DeepSeekModel> {
    println!();
    println!("[2/3] Choose default model:");
    println!("  1) {}  V4 Flash  (recommended)", V4_FLASH.id);
    println!("  2) {}  V4 Pro", V4_PRO.id);
    loop {
        let raw = prompt("> ")?;
        let t = raw.trim();
        if t.is_empty() {
            return Ok(&V4_FLASH);
        }
        match t {
            "1" => return Ok(&V4_FLASH),
            "2" => return Ok(&V4_PRO),
            _ => println!("  enter 1 or 2"),
        }
    }
}

fn ask_user_name() -> Result<String> {
    println!();
    println!("[3/3] What should Aili call you?");
    loop {
        let raw = prompt("> ")?;
        let t = raw.trim().to_string();
        if t.is_empty() {
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
        bail!("aborted (eof)");
    }
    Ok(buf)
}
