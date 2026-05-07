use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use aili::chat::Message;
use aili::config::{self, ResolvedConfig};
use aili::stream::{StreamEvent, StreamOutcome, probe_local, run_stream};
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

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
                None => aili::tui::run(cfg, client).await,
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
    let cancel = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    let (tx, mut rx) = mpsc::channel::<StreamEvent>(64);

    let drain = tokio::spawn(async move {
        let mut out = tokio::io::stdout();
        while let Some(StreamEvent::Token(t)) = rx.recv().await {
            out.write_all(t.as_bytes()).await.ok();
            out.flush().await.ok();
        }
        out.write_all(b"\n").await.ok();
        out.flush().await.ok();
    });

    let outcome = run_stream(client, cfg, &messages, tx, cancel).await?;
    drain.await.ok();
    if outcome == StreamOutcome::Cancelled {
        std::process::exit(130);
    }
    Ok(())
}

