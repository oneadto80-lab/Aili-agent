mod keymap;
mod render;
mod stream_state;

use anyhow::{Context, Result};
use crossterm::cursor::MoveTo;
use crossterm::event::{
    DisableBracketedPaste, EnableBracketedPaste, Event as CtEvent, EventStream, KeyEvent,
    KeyEventKind, KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{Clear, ClearType, disable_raw_mode, enable_raw_mode};
use futures_util::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::widgets::Block;
use ratatui::{Terminal, TerminalOptions, Viewport};
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
const INLINE_HEIGHT: u16 = 8;

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
    pub started: Instant,
    pub chars: usize,
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
        Clear(ClearType::All),
        Clear(ClearType::Purge),
        MoveTo(0, 0),
        EnableBracketedPaste
    )
    .context("prepare terminal screen")?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(INLINE_HEIGHT),
        },
    )
    .context("init inline terminal")?;
    terminal.clear().ok();
    terminal.hide_cursor().ok();
    Ok(terminal)
}

fn restore_terminal(term: &mut Term) -> Result<()> {
    term.clear().ok();
    execute!(term.backend_mut(), DisableBracketedPaste).ok();
    term.show_cursor().ok();
    disable_raw_mode().ok();
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
    pub submit_pending: bool,
    welcome_done: bool,
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
            welcome_done: false,
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
                        Ok(ev) => self.on_terminal_event(ev, term)?,
                        Err(e) => self.flash(format!("input error: {e}")),
                    }
                }
                Some(StreamEvent::Token(t)) = recv_stream(&mut self.in_flight) => {
                    if let Some(f) = self.in_flight.as_mut() {
                        f.chars += t.chars().count();
                    }
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
        let width = term_width(term);
        let lines = render::welcome_page_lines(self, width);
        insert_history_lines(term, lines)?;
        self.welcome_done = true;
        Ok(())
    }

    fn write_user_to_scrollback(&mut self, term: &mut Term, text: &str) -> Result<()> {
        let width = term_width(term);
        let lines = render::user_message_lines(&self.cfg.persona.user_name, text, width);
        insert_history_lines(term, lines)
    }

    fn write_assistant_to_scrollback(&mut self, term: &mut Term, text: &str) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        let width = term_width(term);
        let lines = render::assistant_message_lines(&self.cfg.persona.assistant_name, text, width);
        insert_history_lines(term, lines)
    }

    fn write_note_to_scrollback(&mut self, term: &mut Term, note: &str) -> Result<()> {
        insert_history_lines(term, render::note_lines(note))
    }

    fn clear_terminal_ui(&mut self, term: &mut Term) -> Result<()> {
        execute!(
            term.backend_mut(),
            Clear(ClearType::All),
            Clear(ClearType::Purge)
        )
        .ok();
        term.clear().ok();
        self.history.clear();
        self.stream.drain_all();
        self.welcome_done = false;
        self.write_welcome(term)
    }

    fn on_app_event(&mut self, ev: AppEvent, term: &mut Term) -> Result<()> {
        match ev {
            AppEvent::StreamFinished(res) => {
                self.drain_in_flight_tokens();
                let final_pending = self.stream.drain_all();
                self.append_assistant_text(&final_pending);
                let assistant_text = self
                    .history
                    .last()
                    .filter(|m| m.role == Role::Assistant)
                    .map(|m| m.text.clone())
                    .unwrap_or_default();
                match res {
                    Ok(StreamOutcome::Done) => {
                        self.write_assistant_to_scrollback(term, &assistant_text)?;
                    }
                    Ok(StreamOutcome::Cancelled) => {
                        self.write_assistant_to_scrollback(term, &assistant_text)?;
                        self.write_note_to_scrollback(term, "(cancelled)")?;
                    }
                    Err(e) => {
                        if !assistant_text.is_empty() {
                            self.write_assistant_to_scrollback(term, &assistant_text)?;
                        }
                        self.write_note_to_scrollback(term, &format!("error: {e:#}"))?;
                    }
                }
                self.in_flight = None;
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

    fn on_terminal_event(&mut self, ev: CtEvent, term: &mut Term) -> Result<()> {
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
            CtEvent::Resize(_, _) => self.rewrite_scrollback(term)?,
            _ => {}
        }
        Ok(())
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
                f.chars += t.chars().count();
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

    fn submit(&mut self, term: &mut Term) -> Result<()> {
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
            self.handle_slash(rest, term)?;
            return Ok(());
        }

        self.write_user_to_scrollback(term, &text)?;
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
            started: Instant::now(),
            chars: 0,
        });
        Ok(())
    }

    fn handle_slash(&mut self, rest: &str, term: &mut Term) -> Result<()> {
        let mut parts = rest.split_whitespace();
        let Some(cmd) = parts.next() else {
            return Ok(());
        };
        match cmd {
            "exit" | "quit" => self.quit = true,
            "clear" => {
                self.clear_terminal_ui(term)?;
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
        Ok(())
    }

    fn flush_stream(&mut self, term: &mut Term) -> Result<()> {
        if self.stream.ready() {
            let chunk = self.stream.drain_pending();
            self.append_assistant_text(&chunk);
        }
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

    fn rewrite_scrollback(&mut self, term: &mut Term) -> Result<()> {
        self.drain_in_flight_tokens();
        let pending = self.stream.drain_all();
        self.append_assistant_text(&pending);

        execute!(
            term.backend_mut(),
            Clear(ClearType::All),
            Clear(ClearType::Purge),
            MoveTo(0, 0)
        )
        .ok();
        term.clear().ok();

        self.welcome_done = false;
        self.write_welcome(term)?;
        for msg in self.replayable_history() {
            match msg.role {
                Role::User => self.write_user_to_scrollback(term, &msg.text)?,
                Role::Assistant => self.write_assistant_to_scrollback(term, &msg.text)?,
            }
        }
        Ok(())
    }

    fn replayable_history(&self) -> Vec<UiMessage> {
        let mut history = self.history.clone();
        if self.in_flight.is_some()
            && matches!(history.last(), Some(m) if m.role == Role::Assistant)
        {
            history.pop();
        }
        history
    }
}

fn term_width(term: &mut Term) -> usize {
    term.size().map(|r| r.width as usize).unwrap_or(80).max(1)
}

fn insert_history_lines(term: &mut Term, lines: Vec<ratatui::text::Line<'static>>) -> Result<()> {
    if lines.is_empty() {
        return Ok(());
    }
    let height = lines.len() as u16;
    term.insert_before(height, move |buf| render::paint_lines(buf, lines))
        .context("insert terminal history")
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AltScreenMode, Persona, TuiConfig};
    use crate::provider::Provider;

    fn test_cfg() -> ResolvedConfig {
        ResolvedConfig {
            provider: Provider::DeepSeek,
            base_url: "https://example.test/v1".into(),
            api_key: "test-key".into(),
            model: "deepseek-v4-flash".into(),
            temperature: None,
            top_p: None,
            max_tokens: None,
            stop: Vec::new(),
            persona: Persona {
                user_name: "Rose".into(),
                assistant_name: "Aili".into(),
                description: String::new(),
            },
            tui: TuiConfig {
                alternate_screen: AltScreenMode::Never,
            },
        }
    }

    #[test]
    fn replayable_history_keeps_finalized_messages() {
        let mut app = App::new(test_cfg(), reqwest::Client::new());
        app.history.push(UiMessage {
            role: Role::User,
            text: "hello".into(),
        });
        app.history.push(UiMessage {
            role: Role::Assistant,
            text: "hi".into(),
        });

        let replayable = app.replayable_history();

        assert_eq!(replayable.len(), 2);
        assert_eq!(replayable[1].text, "hi");
    }

    #[test]
    fn replayable_history_skips_streaming_assistant_tail() {
        let mut app = App::new(test_cfg(), reqwest::Client::new());
        app.history.push(UiMessage {
            role: Role::User,
            text: "hello".into(),
        });
        app.history.push(UiMessage {
            role: Role::Assistant,
            text: "partial".into(),
        });
        let (_token_tx, token_rx) = mpsc::channel::<StreamEvent>(1);
        let (cancel_tx, _cancel_rx) = oneshot::channel();
        app.in_flight = Some(InFlight {
            token_rx,
            cancel_tx: Some(cancel_tx),
            started: Instant::now(),
            chars: 0,
        });

        let replayable = app.replayable_history();

        assert_eq!(replayable.len(), 1);
        assert_eq!(replayable[0].role, Role::User);
    }
}
