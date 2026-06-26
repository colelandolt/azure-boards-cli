use crate::error::CliError;
use std::io::{BufRead, IsTerminal, Write};

/// How dangerous a confirmation-gated operation is.
pub enum Danger {
    /// Recoverable destructive operation (recycle-bin delete, relation remove).
    Destructive,
    /// Irreversible. `--yes` alone is NOT sufficient; the caller must have
    /// verified `--destroy --confirm-id <id>` before calling confirm().
    Permanent,
}

/// Confirmation gate. Rules:
/// - `--dry-run` paths must short-circuit before calling this.
/// - `yes == true` skips the prompt (it means "skip confirmation", never
///   "make the operation more destructive").
/// - Non-TTY without --yes fails closed with exit 9 and never blocks.
pub fn confirm(yes: bool, danger: Danger, summary: &str) -> Result<(), CliError> {
    if yes {
        return Ok(());
    }
    let interactive = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    if !interactive {
        return Err(CliError::UnsafeBlocked(format!(
            "{summary}: confirmation required; re-run with --yes"
        )));
    }
    let stderr = std::io::stderr();
    let mut err = stderr.lock();
    let suffix = match danger {
        Danger::Destructive => "[y/N] ",
        Danger::Permanent => "[y/N] (PERMANENT — cannot be undone) ",
    };
    write!(err, "{summary} {suffix}").ok();
    err.flush().ok();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).ok();
    let answer = line.trim().to_ascii_lowercase();
    if answer == "y" || answer == "yes" {
        Ok(())
    } else {
        Err(CliError::General("operation cancelled".into()))
    }
}

/// Validate the `--destroy --confirm-id` pairing for permanent deletion.
/// Returns UnsafeBlocked (exit 9) on any mismatch, even with --yes.
pub fn require_confirm_id(id: i64, confirm_id: Option<i64>) -> Result<(), CliError> {
    match confirm_id {
        Some(c) if c == id => Ok(()),
        Some(c) => Err(CliError::UnsafeBlocked(format!(
            "--confirm-id {c} does not match target work item {id}"
        ))),
        None => Err(CliError::UnsafeBlocked(format!(
            "permanent destroy requires --confirm-id {id}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirm_id_must_match() {
        assert!(require_confirm_id(5, Some(5)).is_ok());
        assert_eq!(require_confirm_id(5, Some(6)).unwrap_err().exit_code(), 9);
        assert_eq!(require_confirm_id(5, None).unwrap_err().exit_code(), 9);
    }

    #[test]
    fn yes_skips_prompt() {
        assert!(confirm(true, Danger::Destructive, "delete 5").is_ok());
    }

    #[test]
    fn non_tty_without_yes_blocks_with_exit_9() {
        // Test runs without a TTY, so this exercises the fail-closed path.
        if !std::io::stdin().is_terminal() {
            let e = confirm(false, Danger::Destructive, "delete 5").unwrap_err();
            assert_eq!(e.exit_code(), 9);
        }
    }
}
