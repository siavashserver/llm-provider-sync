//! Small terminal helpers shared by the interactive flows and the commands.

use std::io::{self, IsTerminal};

use cliclack::{Theme, ThemeState};
use console::Style;

use crate::error::Error;

/// Branded cliclack theme: blue sidebar/symbols, green on submit.
pub struct BrandTheme;

impl Theme for BrandTheme {
    fn bar_color(&self, _state: &ThemeState) -> Style {
        Style::new().blue()
    }
}

/// Applies the branded cliclack theme. Call once at the start of an interactive run.
pub fn apply_theme() {
    cliclack::set_theme(BrandTheme);
}

/// Prompts need a real terminal on both ends.
pub fn is_interactive() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

pub fn require_interactive() -> Result<(), Error> {
    if is_interactive() {
        Ok(())
    } else {
        Err(Error::Usage(
            "interactive mode needs a terminal; pass a command instead (see --help)".into(),
        ))
    }
}

/// Converts a prompt result, mapping Ctrl-C / Esc to [`Error::Cancelled`].
pub fn prompt<T>(result: io::Result<T>) -> Result<T, Error> {
    result.map_err(|err| {
        if err.kind() == io::ErrorKind::Interrupted {
            Error::Cancelled
        } else {
            Error::Ui(err)
        }
    })
}
