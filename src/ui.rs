//! Small terminal helpers shared by the interactive flows and the commands.

use std::io::{self, IsTerminal};

use crate::error::Error;

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
