use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::io::Write;

use aili::chat::Message;
use aili::config::{self, ResolvedConfig};
use aili::stream::{StreamOutcome, probe_local, run_stream};

#[derive(Parser, Debug)]
#[command(name = "aili", version, about = "Memory-first companion agent (v0.1: chat-only MVP)")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Manage configuration.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Start an interactive chat (or run a single turn with --once).
    Chat {
        /// Run a single turn with the given prompt and exit.
        #[arg(long)]
        once: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum ConfigAction {
    /// Write a default config template to ~/.config/aili/config.toml.
    Init,
    /// Print the resolved config path.
    Path,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli).await {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<()> {
    match cli.cmd {
        Cmd::Config { action } => match action {
            ConfigAction::Init => {
                let path = config::config_path()?;
                config::write_template(&path)?;
                println!("wrote config template to {}", path.display());
                println!("set your API key env var (e.g. export DEEPSEEK_API_KEY=sk-...) and run `aili chat`.");
                Ok(())
            }
            ConfigAction::Path => {
                println!("{}", config::config_path()?.display());
                Ok(())
            }
        },
        Cmd::Chat { once } => {
            let cfg = config::load()?;
            let client = build_client()?;
            probe_local(&client, &cfg).await?;
            match once {
                Some(prompt) => once_turn(&client, &cfg, prompt).await,
                None => repl(&client, &cfg).await,
            }
        }
    }
}

fn build_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(concat!("aili/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("failed to build HTTP client")
}

async fn once_turn(client: &reqwest::Client, cfg: &ResolvedConfig, prompt: String) -> Result<()> {
    let messages = vec![Message::user(prompt)];
    let mut sink = String::new();
    let cancel = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    let outcome = run_stream(client, cfg, &messages, &mut sink, cancel).await?;
    if outcome == StreamOutcome::Cancelled {
        std::process::exit(130);
    }
    Ok(())
}

async fn repl(client: &reqwest::Client, cfg: &ResolvedConfig) -> Result<()> {
    let mut cfg = cfg.clone();
    let mut history: Vec<Message> = Vec::new();
    println!(
        "aili v{} — provider: {}  model: {}",
        env!("CARGO_PKG_VERSION"),
        cfg.provider.as_str(),
        cfg.model
    );
    println!("type your message, /help for commands, /exit to quit.");
    let stdin = std::io::stdin();
    loop {
        print!("\n› ");
        std::io::stdout().flush().ok();
        let mut line = String::new();
        let n = stdin.read_line(&mut line)?;
        if n == 0 {
            println!();
            return Ok(());
        }
        let line = line.trim_end_matches(['\n', '\r']).to_string();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix('/') {
            match handle_slash(rest, &mut cfg, &mut history) {
                SlashResult::Continue => continue,
                SlashResult::Exit => return Ok(()),
            }
        }
        history.push(Message::user(line));
        let mut sink = String::new();
        let cancel = async {
            let _ = tokio::signal::ctrl_c().await;
        };
        let outcome = match run_stream(client, &cfg, &history, &mut sink, cancel).await {
            Ok(o) => o,
            Err(e) => {
                history.pop();
                eprintln!("error: {e:#}");
                continue;
            }
        };
        if outcome == StreamOutcome::Cancelled {
            history.pop();
            eprintln!("(cancelled)");
            continue;
        }
        if !sink.is_empty() {
            history.push(Message::assistant(sink));
        } else {
            history.pop();
        }
    }
}

enum SlashResult {
    Continue,
    Exit,
}

fn handle_slash(rest: &str, cfg: &mut ResolvedConfig, history: &mut Vec<Message>) -> SlashResult {
    let mut parts = rest.split_whitespace();
    let Some(cmd) = parts.next() else {
        return SlashResult::Continue;
    };
    match cmd {
        "exit" | "quit" => return SlashResult::Exit,
        "clear" => {
            history.clear();
            println!("(history cleared)");
        }
        "model" => match parts.next() {
            Some(m) => {
                cfg.model = m.to_string();
                println!("model -> {}", cfg.model);
            }
            None => println!("model: {}", cfg.model),
        },
        "provider" => {
            println!("provider: {}  base_url: {}", cfg.provider.as_str(), cfg.base_url);
        }
        "params" => {
            println!(
                "temperature={:?}  top_p={:?}  max_tokens={:?}  stop={:?}",
                cfg.temperature, cfg.top_p, cfg.max_tokens, cfg.stop
            );
        }
        "help" => {
            println!("/model [name]   show or set model");
            println!("/provider       show provider + base_url");
            println!("/params         show sampling params");
            println!("/clear          clear conversation history");
            println!("/exit           quit");
        }
        other => {
            println!("unknown command: /{other} (try /help)");
        }
    }
    SlashResult::Continue
}
