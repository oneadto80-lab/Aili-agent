mod keymap;
mod render;
mod stream_state;

use anyhow::{Context, Result};
use crossterm::event::{
    DisableBracketedPaste, EnableBracketedPaste, Event as CtEvent, EventStream, KeyEvent,
    KeyEventKind, KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use futures_util::StreamExt;
use ratatui::Terminal;
use ratatui::Viewport;
use ratatui::TerminalOptions;
use ratatui::backend::CrosstermBackend;
use ratatui::widgets::Block;
use std::io::{Stdout, stdout};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};
use tui_textarea::{Input, Key, TextArea};

use crate::chat::Message;
use crate::config::ResolvedConfig;
use crate::stream::{StreamEvent, StreamOutcome, run_stream};

use self::keymap::{Action, KeyContext};
use self::stream_state::StreamState;

type Term = Terminal<CrosstermBackend<Stdout>>;

const INLINE_HEIGHT: u16 = 5;

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
    execute!(stdout(), EnableBracketedPaste).context("enable bracketed paste")?;
    let backend = CrosstermBackend::new(stdout());
    Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(INLINE_HEIGHT),
        },
    )
    .context("init terminal")
}

fn restore_terminal(term: &mut Term) -> Result<()> {
    execute!(term.backend_mut(), DisableBracketedPaste).ok();
    disable_raw_mode().ok();
    term.show_cursor().ok();
    println!();
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
    /// Bytes of the current assistant response not yet committed to scrollback.
    pub live_buffer: String,
    pub welcome_done: bool,
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
            live_buffer: String::new(),
            welcome_done: false,
            submit_pending: false,
        }
    }

    async fn run(mut self, term: &mut Term) -> Result<()> {
        self.write_welcome(term)?;
        let mut events = EventStream::new();
        let mut tick = tokio::time::interval(Duration::from_millis(50));

        loop {
            self.flush_stream(term)?;
            self.expire_timed();
            term.draw(|f| render::draw_inline(&self, f))?;
            if self.quit {
                break;
            }
            tokio::select! {
                biased;
                Some(app_ev) = self.app_rx.recv() => {
                    self.on_app_event(app_ev, term)?;
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

    fn write_welcome(&mut self, term: &mut Term) -> Result<()> {
        if self.welcome_done {
            return Ok(());
        }
        let lines = render::welcome_card_lines(&self.cfg.model, env!("CARGO_PKG_VERSION"));
        let h = lines.len() as u16;
        term.insert_before(h, |buf| render::paint_lines(buf, lines))
            .context("insert welcome")?;
        self.welcome_done = true;
        Ok(())
    }

    fn write_user_to_scrollback(&mut self, term: &mut Term, text: &str) -> Result<()> {
        let width = term.size()?.width as usize;
        let lines = render::user_message_lines(text, width);
        let h = lines.len() as u16;
        term.insert_before(h, |buf| render::paint_lines(buf, lines))
            .context("insert user message")?;
        Ok(())
    }

    fn write_assistant_header(&mut self, term: &mut Term) -> Result<()> {
        let lines = render::assistant_header_lines();
        let h = lines.len() as u16;
        term.insert_before(h, |buf| render::paint_lines(buf, lines))
            .context("insert assistant header")?;
        Ok(())
    }

    fn write_assistant_chunk(&mut self, term: &mut Term, text: &str) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        let width = term.size()?.width as usize;
        let lines = render::assistant_body_lines(text, width);
        let h = lines.len() as u16;
        if h == 0 {
            return Ok(());
        }
        term.insert_before(h, |buf| render::paint_lines(buf, lines))
            .context("insert assistant chunk")?;
        Ok(())
    }

    fn write_note(&mut self, term: &mut Term, note: &str) -> Result<()> {
        let lines = render::note_lines(note);
        let h = lines.len() as u16;
        term.insert_before(h, |buf| render::paint_lines(buf, lines))
            .context("insert note")?;
        Ok(())
    }

    fn write_separator(&mut self, term: &mut Term) -> Result<()> {
        let lines = render::turn_separator();
        let h = lines.len() as u16;
        term.insert_before(h, |buf| render::paint_lines(buf, lines))
            .context("insert separator")?;
        Ok(())
    }

    fn on_app_event(&mut self, ev: AppEvent, term: &mut Term) -> Result<()> {
        match ev {
            AppEvent::StreamFinished(res) => {
                // Drain any final batched tokens.
                self.flush_stream(term)?;
                // Whatever's left in live_buffer (no trailing \n) goes to scrollback.
                if !self.live_buffer.is_empty() {
                    let tail = std::mem::take(&mut self.live_buffer);
                    if let Some(last) = self.history.last_mut() {
                        if last.role == Role::Assistant {
                            last.text.push_str(&tail);
                        }
                    }
                    self.write_assistant_chunk(term, &tail)?;
                }
                self.in_flight = None;
                match res {
                    Ok(StreamOutcome::Done) => self.write_separator(term)?,
                    Ok(StreamOutcome::Cancelled) => {
                        self.write_note(term, "(cancelled)")?;
                    }
                    Err(e) => {
                        self.write_note(term, &format!("error: {e:#}"))?;
                        if matches!(
                            self.history.last(),
                            Some(m) if m.role == Role::Assistant && m.text.is_empty()
                        ) {
                            self.history.pop();
                        }
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
        if let Some(f) = self.in_flight.as_mut() {
            if let Some(tx) = f.cancel_tx.take() {
                let _ = tx.send(());
            }
            // Drain whatever is already buffered in the channel so it isn't lost.
            while let Ok(StreamEvent::Token(t)) = f.token_rx.try_recv() {
                self.stream.enqueue(t);
            }
        }
    }

    fn submit(&mut self, term: &mut Term) -> Result<()> {
        if self.in_flight.is_some() {
            return Ok(());
        }
        let text = self.composer.lines().join("\n").trim().to_string();
        if text.is_empty() {
            return Ok(());
        }
        self.composer = fresh_composer();

        if let Some(rest) = text.strip_prefix('/') {
            self.handle_slash(rest);
            return Ok(());
        }

        // Push user to scrollback immediately and to history for context.
        self.write_user_to_scrollback(term, &text)?;
        self.history.push(UiMessage { role: Role::User, text: text.clone() });
        self.history.push(UiMessage { role: Role::Assistant, text: String::new() });

        // Header for the assistant turn — body lines stream in below.
        self.write_assistant_header(term)?;

        let messages: Vec<Message> = self
            .history
            .iter()
            .filter(|m| m.role == Role::User || (m.role == Role::Assistant && !m.text.is_empty()))
            .map(|m| match m.role {
                Role::User => Message::user(m.text.clone()),
                Role::Assistant => Message::assistant(m.text.clone()),
            })
            .collect();

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
                self.flash("(history cleared — terminal scrollback unaffected)".to_string());
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
            "help" => self.flash(
                "/model [name] /provider /params /clear /exit  ·  scroll: trackpad/wheel/Cmd+↑↓".to_string()
            ),
            other => self.flash(format!("unknown command: /{other} (try /help)")),
        }
    }

    /// Drain ready-to-commit lines from `live_buffer` into scrollback. A line
    /// is "ready" when it's terminated by `\n`. Anything trailing without a
    /// newline stays in the buffer until the stream finishes.
    fn flush_stream(&mut self, term: &mut Term) -> Result<()> {
        // Pull batched tokens out of the StreamState policy first.
        if self.stream.ready() {
            let chunk = self.stream.drain_pending();
            self.live_buffer.push_str(&chunk);
        }
        // Commit each completed line.
        while let Some(idx) = self.live_buffer.find('\n') {
            let line: String = self.live_buffer.drain(..=idx).collect();
            if let Some(last) = self.history.last_mut() {
                if last.role == Role::Assistant {
                    last.text.push_str(&line);
                }
            }
            self.write_assistant_chunk(term, line.trim_end_matches('\n'))?;
        }

        // Handle deferred submit (Enter): we couldn't call `submit` from
        // on_key because it needs &mut Term.
        if self.submit_pending {
            self.submit_pending = false;
            self.submit(term)?;
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
    t.set_placeholder_text("type your message — Enter to send, Shift+Enter for newline");
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
