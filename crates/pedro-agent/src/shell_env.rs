//! Recovering the user's real `PATH`.
//!
//! A GUI application launched from Finder or Dock inherits `launchd`'s
//! environment, not the one the user's shell builds in `.zshrc` / `.profile`.
//! Agent CLIs are almost always installed into a directory that only exists in
//! the shell's `PATH` (`~/.local/bin`, a Node version manager's shim
//! directory, ...), so we ask the login shell what it thinks `PATH` is.

use std::process::Command;
use std::time::Duration;

use crate::process::capture_stdout;

/// Printed around the value so that anything an interactive rc file writes to
/// stdout can be filtered out.
const MARKER: &str = "__PEDRO_PATH__";

const TIMEOUT: Duration = Duration::from_secs(3);

/// Asks the user's login shell for its `PATH`.
///
/// Returns `None` when there is no `SHELL`, the shell fails to start, or it
/// does not answer within [`TIMEOUT`].
pub fn login_shell_path() -> Option<String> {
    let shell = std::env::var("SHELL").ok()?;
    if shell.is_empty() {
        return None;
    }

    let script = format!("printf '{MARKER}%s{MARKER}' \"$PATH\"");

    let mut command = Command::new(&shell);
    // `-l` sources the login files, `-i` the interactive ones; many users only
    // extend PATH in the interactive file. `-c` runs our one-liner.
    command.args(["-lic", &script]);

    let output = capture_stdout(command, TIMEOUT)?;
    extract_marked_value(&output).map(str::to_owned)
}

/// Pulls the value written between the two markers out of noisy shell output.
fn extract_marked_value(output: &str) -> Option<&str> {
    let (_, rest) = output.split_once(MARKER)?;
    let (value, _) = rest.split_once(MARKER)?;
    if value.is_empty() { None } else { Some(value) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_the_value_between_markers() {
        let output = format!("{MARKER}/usr/bin:/bin{MARKER}");
        assert_eq!(extract_marked_value(&output), Some("/usr/bin:/bin"));
    }

    #[test]
    fn ignores_noise_written_by_rc_files() {
        let output = format!("welcome!\n{MARKER}/opt/homebrew/bin{MARKER}");
        assert_eq!(extract_marked_value(&output), Some("/opt/homebrew/bin"));
    }

    #[test]
    fn rejects_output_without_markers() {
        assert_eq!(extract_marked_value("/usr/bin:/bin"), None);
    }

    #[test]
    fn rejects_an_empty_path() {
        let output = format!("{MARKER}{MARKER}");
        assert_eq!(extract_marked_value(&output), None);
    }
}
