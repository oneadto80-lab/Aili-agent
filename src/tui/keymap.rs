use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// What state the app is in when a key arrives. Used to disambiguate keys
/// that mean different things in different contexts (e.g. Ctrl-C).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyContext {
    /// A network stream is currently delivering tokens.
    Streaming,
    /// Idle, and the user recently pressed Ctrl-C (within the quit-arm window).
    IdleArmed,
    /// Idle, no quit-arm active.
    IdleUnarmed,
    /// A dialog overlay is active (model select, help, etc).
    Dialog,
}

/// What the app should *do* in response to a key. Decoupled from the key
/// itself so that future configurable keymaps can swap the resolver without
/// touching the App loop.
///
/// Note: scrolling actions are intentionally absent in the main chat. Finalized
/// history is written to terminal scrollback, so the terminal owns review.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Submit,
    InsertNewline,
    CancelStream,
    QuitArm,
    QuitNow,
    /// Pass the key event through to the composer (typing, cursor movement,
    /// editing — anything tui-textarea handles by default).
    ForwardToComposer,
    /// Scroll the fullscreen session message history.
    ScrollUp,
    ScrollDown,
    ScrollPageUp,
    ScrollPageDown,
    ScrollTop,
    ScrollBottom,
    /// Dialog navigation
    DialogUp,
    DialogDown,
    DialogConfirm,
    DialogClose,
}

/// Default keybindings. The function signature is the seam for future
/// user-configurable bindings.
pub fn resolve(k: &KeyEvent, ctx: KeyContext) -> Action {
    let m = k.modifiers;
    let ctrl = m.contains(KeyModifiers::CONTROL);
    let shift = m.contains(KeyModifiers::SHIFT);
    let alt = m.contains(KeyModifiers::ALT);

    // Dialog mode
    if ctx == KeyContext::Dialog {
        return match k.code {
            KeyCode::Esc | KeyCode::Char('q') => Action::DialogClose,
            KeyCode::Up | KeyCode::Char('k') => Action::DialogUp,
            KeyCode::Down | KeyCode::Char('j') => Action::DialogDown,
            KeyCode::Enter => Action::DialogConfirm,
            _ => Action::ForwardToComposer,
        };
    }

    if ctrl && k.code == KeyCode::Char('c') {
        return match ctx {
            KeyContext::Streaming => Action::CancelStream,
            KeyContext::IdleArmed => Action::QuitNow,
            KeyContext::IdleUnarmed => Action::QuitArm,
            KeyContext::Dialog => Action::DialogClose,
        };
    }
    if ctrl && k.code == KeyCode::Char('d') {
        return if ctx == KeyContext::Dialog {
            Action::DialogClose
        } else {
            Action::QuitNow
        };
    }
    if k.code == KeyCode::Enter {
        if shift || alt {
            return Action::InsertNewline;
        }
        return Action::Submit;
    }

    // Scroll keys for fullscreen session mode
    match k.code {
        KeyCode::Up | KeyCode::Char('k') if ctrl => return Action::ScrollUp,
        KeyCode::Down | KeyCode::Char('j') if ctrl => return Action::ScrollDown,
        KeyCode::PageUp => return Action::ScrollPageUp,
        KeyCode::PageDown => return Action::ScrollPageDown,
        KeyCode::Home => return Action::ScrollTop,
        KeyCode::End => return Action::ScrollBottom,
        _ => {}
    }

    Action::ForwardToComposer
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn ctrl_c_streaming_cancels() {
        let r = resolve(
            &key(KeyCode::Char('c'), KeyModifiers::CONTROL),
            KeyContext::Streaming,
        );
        assert_eq!(r, Action::CancelStream);
    }

    #[test]
    fn esc_closes_dialog() {
        let r = resolve(&key(KeyCode::Esc, KeyModifiers::NONE), KeyContext::Dialog);
        assert_eq!(r, Action::DialogClose);
    }

    #[test]
    fn ctrl_c_idle_arms_then_quits() {
        let r1 = resolve(
            &key(KeyCode::Char('c'), KeyModifiers::CONTROL),
            KeyContext::IdleUnarmed,
        );
        assert_eq!(r1, Action::QuitArm);
        let r2 = resolve(
            &key(KeyCode::Char('c'), KeyModifiers::CONTROL),
            KeyContext::IdleArmed,
        );
        assert_eq!(r2, Action::QuitNow);
    }

    #[test]
    fn enter_submits_shift_enter_newlines() {
        assert_eq!(
            resolve(
                &key(KeyCode::Enter, KeyModifiers::NONE),
                KeyContext::IdleUnarmed
            ),
            Action::Submit
        );
        assert_eq!(
            resolve(
                &key(KeyCode::Enter, KeyModifiers::SHIFT),
                KeyContext::IdleUnarmed
            ),
            Action::InsertNewline
        );
    }

    #[test]
    fn ctrl_d_quits() {
        assert_eq!(
            resolve(
                &key(KeyCode::Char('d'), KeyModifiers::CONTROL),
                KeyContext::IdleUnarmed
            ),
            Action::QuitNow
        );
    }

    #[test]
    fn unknown_keys_forward_to_composer() {
        assert_eq!(
            resolve(
                &key(KeyCode::Char('x'), KeyModifiers::NONE),
                KeyContext::IdleUnarmed
            ),
            Action::ForwardToComposer
        );
    }
}
