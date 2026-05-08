//! Welcome-screen text logo.

pub const WELCOME_LOGO: &str = "\
U^ｪ^U";

pub fn lines() -> impl Iterator<Item = &'static str> {
    WELCOME_LOGO.lines()
}
