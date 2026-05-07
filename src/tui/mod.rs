mod keymap;
mod render;
mod stream_state;

use anyhow::{Context, Result};
use crossterm::event::{
    DisableBracketedPaste, EnableBracketedPaste, Event as CtEvent, EventStream, KeyEvent,
    KeyEventKind, KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{
    Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use futures_util::StreamExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::widgets::Block;
use std::io::{Stdout, stdout};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};
use tui_textarea::{Input, Key, TextArea};

use crate::chat::{Message, prepend_system_prompt};
use crate::config::ResolvedConfig;
use crate::stream::{StreamEvent, StreamOutcome, run_stream};

use self::keymap::{Action, KeyContext};
use self::stream_state::StreamState;

type Term = Terminal<CrosstermBackend<Stdout>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone)]
pub(crate) struct UiMessage {
    pub role: Role,
    pub text: String,
}

#[derive(Debug)]
pub(crate) enum AppEvent {
    StreamFinished(anyhow::Result<StreamOutcome>),
}

pub(crate) struct InFlight {
    token_rx: mpsc::Receiver<StreamEvent>,
    cancel_tx: Option<oneshot::Sender<()>>,
}

pub(crate) struct CtrlCArm {
    expires: Instant,
}

pub async fn run(cfg: ResolvedConfig, client: reqwest::Client) -> Result<()> {
    let mut term = setup_terminal()?;
    let res = App::new(cfg, client).run(&mut term).await;
    restore_terminal(&mut term)?;
    res
}

fn setup_terminal() -> Result<Term> {
    enable_raw_mode().context("enable raw mode")?;
    execute!(
        stdout(),
        EnterAlternateScreen,
        Clear(ClearType::All),
        EnableBracketedPaste
    )
    .context("enter fullscreen terminal")?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend).context("init terminal")?;
    terminal.clear().ok();
    terminal.hide_cursor().ok();
    Ok(terminal)
}

fn restore_terminal(term: &mut Term) -> Result<()> {
    term.clear().ok();
    execute!(
        term.backend_mut(),
        DisableBracketedPaste,
        Clear(ClearType::All),
        LeaveAlternateScreen
    )
    .ok();
    disable_raw_mode().ok();
    term.show_cursor().ok();
    Ok(())
}

pub(crate) struct App {
    pub cfg: ResolvedConfig,
    pub client: reqwest::Client,
    pub history: Vec<UiMessage>,
    pub composer: TextArea<'static>,
    pub stream: StreamState,
    pub in_flight: Option<InFlight>,
    pub app_tx: mpsc::UnboundedSender<AppEvent>,
    pub app_rx: mpsc::UnboundedReceiver<AppEvent>,
    pub quit: bool,
    pub ctrl_c_arm: Option<CtrlCArm>,
    pub status_msg: Option<(String, Instant)>,
    pub submit_pending: bool,
}

impl App {
    fn new(cfg: ResolvedConfig, client: reqwest::Client) -> Self {
        let (app_tx, app_rx) = mpsc::unbounded_channel();
        Self {
            cfg,
            client,
            history: Vec::new(),
            composer: fresh_composer(),
            stream: StreamState::new(),
            in_flight: None,
            app_tx,
            app_rx,
            quit: false,
            ctrl_c_arm: None,
            status_msg: None,
            submit_pending: false,
        }
    }

    async fn run(mut self, term: &mut Term) -> Result<()> {
        let mut events = EventStream::new();
        let mut tick = tokio::time::interval(Duration::from_millis(50));

        loop {
            self.flush_stream()?;
            self.expire_timed();
            term.draw(|f| render::draw_fullscreen(&self, f))?;
            if self.quit {
                break;
            }
            tokio::select! {
                biased;
                Some(app_ev) = self.app_rx.recv() => {
                    self.on_app_event(app_ev)?;
                }
                Some(ev) = events.next() => {
                    match ev {
                        Ok(ev) => self.on_terminal_event(ev),
                        Err(e) => self.flash(format!("input error: {e}")),
                    }
                }
                Some(StreamEvent::Token(t)) = recv_stream(&mut self.in_flight) => {
                    self.stream.enqueue(t);
                }
                _ = tick.tick() => {}
            }
        }
        Ok(())
    }

    fn on_app_event(&mut self, ev: AppEvent) -> Result<()> {
        match ev {
            AppEvent::StreamFinished(res) => {
                self.drain_in_flight_tokens();
                let final_pending = self.stream.drain_all();
                self.append_assistant_text(&final_pending);
                self.in_flight = None;
                match res {
                    Ok(StreamOutcome::Done) => {}
                    Ok(StreamOutcome::Cancelled) => self.append_assistant_text("\n(cancelled)"),
                    Err(e) => {
                        if matches!(
                            self.history.last(),
                            Some(m) if m.role == Role::Assistant && m.text.is_empty()
                        ) {
                            self.history.pop();
                        }
                        self.history.push(UiMessage {
                            role: Role::Assistant,
                            text: format!("error: {e:#}"),
                        });
                    }
                }
                if matches!(
                    self.history.last(),
                    Some(m) if m.role == Role::Assistant && m.text.is_empty()
                ) {
                    self.history.pop();
                }
            }
        }
        Ok(())
    }

    fn on_terminal_event(&mut self, ev: CtEvent) {
        match ev {
            CtEvent::Key(k) if k.kind == KeyEventKind::Press => self.on_key(k),
            CtEvent::Paste(p) => {
                for ch in p.chars() {
                    self.composer.input(Input {
                        key: Key::Char(ch),
                        ctrl: false,
                        alt: false,
                        shift: false,
                    });
                }
            }
            _ => {}
        }
    }

    fn on_key(&mut self, k: KeyEvent) {
        let ctx = if self.in_flight.is_some() {
            KeyContext::Streaming
        } else if self
            .ctrl_c_arm
            .as_ref()
            .map(|a| a.expires > Instant::now())
            .unwrap_or(false)
        {
            KeyContext::IdleArmed
        } else {
            KeyContext::IdleUnarmed
        };
        match keymap::resolve(&k, ctx) {
            Action::Submit => self.submit_pending = true,
            Action::InsertNewline => {
                self.composer.input(Input {
                    key: Key::Enter,
                    ctrl: false,
                    alt: false,
                    shift: false,
                });
            }
            Action::CancelStream => self.cancel_stream(),
            Action::QuitArm => {
                self.ctrl_c_arm = Some(CtrlCArm {
                    expires: Instant::now() + Duration::from_millis(1500),
                });
                self.flash("press Ctrl-C again to quit".to_string());
            }
            Action::QuitNow => self.quit = true,
            Action::ForwardToComposer => {
                let input = ct_to_input(&k);
                self.composer.input(input);
            }
        }
    }

    fn cancel_stream(&mut self) {
        let mut should_drain = false;
        if let Some(f) = self.in_flight.as_mut() {
            if let Some(tx) = f.cancel_tx.take() {
                let _ = tx.send(());
            }
            should_drain = true;
        }
        if should_drain {
            self.drain_in_flight_tokens();
        }
    }

    fn drain_in_flight_tokens(&mut self) {
        if let Some(f) = self.in_flight.as_mut() {
            while let Ok(StreamEvent::Token(t)) = f.token_rx.try_recv() {
                self.stream.enqueue(t);
            }
        }
    }

    fn append_assistant_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if let Some(last) = self.history.last_mut() {
            if last.role == Role::Assistant {
                last.text.push_str(text);
                return;
            }
        }
        self.history.push(UiMessage {
            role: Role::Assistant,
            text: text.to_string(),
        });
    }

    fn submit(&mut self) -> Result<()> {
        if self.in_flight.is_some() {
            return Ok(());
        }
        let _ = self.stream.drain_all();
        let text = self.composer.lines().join("\n").trim().to_string();
        if text.is_empty() {
            return Ok(());
        }
        self.composer = fresh_composer();

        if let Some(rest) = text.strip_prefix('/') {
            self.handle_slash(rest);
            return Ok(());
        }

        self.history.push(UiMessage {
            role: Role::User,
            text: text.clone(),
        });
        self.history.push(UiMessage {
            role: Role::Assistant,
            text: String::new(),
        });

        let messages = prepend_system_prompt(
            &self.cfg,
            self.history
                .iter()
                .filter(|m| {
                    m.role == Role::User || (m.role == Role::Assistant && !m.text.is_empty())
                })
                .map(|m| match m.role {
                    Role::User => Message::user(m.text.clone()),
                    Role::Assistant => Message::assistant(m.text.clone()),
                })
                .collect(),
        );

        let (token_tx, token_rx) = mpsc::channel::<StreamEvent>(64);
        let (cancel_tx, cancel_rx) = oneshot::channel();
        let cfg = self.cfg.clone();
        let client = self.client.clone();
        let app_tx = self.app_tx.clone();
        tokio::spawn(async move {
            let cancel = async {
                let _ = cancel_rx.await;
            };
            let res = run_stream(&client, &cfg, &messages, token_tx, cancel).await;
            let _ = app_tx.send(AppEvent::StreamFinished(res));
        });
        self.in_flight = Some(InFlight {
            token_rx,
            cancel_tx: Some(cancel_tx),
        });
        Ok(())
    }

    fn handle_slash(&mut self, rest: &str) {
        let mut parts = rest.split_whitespace();
        let Some(cmd) = parts.next() else { return };
        match cmd {
            "exit" | "quit" => self.quit = true,
            "clear" => {
                self.history.clear();
                self.flash("(history cleared)".to_string());
            }
            "model" => match parts.next() {
                Some(m) => {
                    self.cfg.model = m.to_string();
                    self.flash(format!("model -> {}", self.cfg.model));
                }
                None => self.flash(format!("model: {}", self.cfg.model)),
            },
            "provider" => self.flash(format!(
                "provider: {}  base_url: {}",
                self.cfg.provider.as_str(),
                self.cfg.base_url
            )),
            "params" => self.flash(format!(
                "temperature={:?}  top_p={:?}  max_tokens={:?}",
                self.cfg.temperature, self.cfg.top_p, self.cfg.max_tokens
            )),
            "help" => self.flash("/model [name] /provider /params /clear /exit".to_string()),
            other => self.flash(format!("unknown command: /{other} (try /help)")),
        }
    }

    fn flush_stream(&mut self) -> Result<()> {
        if self.stream.ready() {
            let chunk = self.stream.drain_pending();
            self.append_assistant_text(&chunk);
        }
        if self.submit_pending {
            self.submit_pending = false;
            self.submit()?;
        }
        Ok(())
    }

    fn expire_timed(&mut self) {
        if let Some(arm) = &self.ctrl_c_arm {
            if arm.expires <= Instant::now() {
                self.ctrl_c_arm = None;
            }
        }
        if let Some((_, at)) = &self.status_msg {
            if at.elapsed() > Duration::from_secs(3) {
                self.status_msg = None;
            }
        }
    }

    fn flash(&mut self, msg: String) {
        self.status_msg = Some((msg, Instant::now()));
    }
}

fn fresh_composer() -> TextArea<'static> {
    let mut t = TextArea::default();
    t.set_block(Block::default());
    t
}

async fn recv_stream(in_flight: &mut Option<InFlight>) -> Option<StreamEvent> {
    match in_flight {
        Some(f) => f.token_rx.recv().await,
        None => std::future::pending().await,
    }
}

fn ct_to_input(k: &KeyEvent) -> Input {
    use crossterm::event::KeyCode;
    let key = match k.code {
        KeyCode::Char(c) => Key::Char(c),
        KeyCode::Enter => Key::Enter,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Delete => Key::Delete,
        KeyCode::Tab => Key::Tab,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::Esc => Key::Esc,
        _ => Key::Null,
    };
    Input {
        key,
        ctrl: k.modifiers.contains(KeyModifiers::CONTROL),
        alt: k.modifiers.contains(KeyModifiers::ALT),
        shift: k.modifiers.contains(KeyModifiers::SHIFT),
    }
}
