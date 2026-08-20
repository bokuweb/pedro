//! Finding coding agent CLIs that are already installed on this machine.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::process::capture_stdout;
use crate::shell_env::login_shell_path;

const VERSION_TIMEOUT: Duration = Duration::from_secs(5);

/// A coding agent CLI that pedro knows how to talk to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentKind {
    ClaudeCode,
    Codex,
}

impl AgentKind {
    pub const ALL: [AgentKind; 2] = [AgentKind::ClaudeCode, AgentKind::Codex];

    /// The executable name to look for on `PATH`.
    pub fn program(self) -> &'static str {
        match self {
            AgentKind::ClaudeCode => "claude",
            AgentKind::Codex => "codex",
        }
    }

    /// The name shown in the UI.
    pub fn display_name(self) -> &'static str {
        match self {
            AgentKind::ClaudeCode => "Claude Code",
            AgentKind::Codex => "Codex CLI",
        }
    }

    /// Installation locations that are commonly missing from `PATH`.
    fn extra_locations(self, home: &Path) -> Vec<PathBuf> {
        match self {
            // The `claude` installer offers a self-contained install that is
            // not linked into any bin directory.
            AgentKind::ClaudeCode => vec![home.join(".claude/local/claude")],
            AgentKind::Codex => vec![],
        }
    }
}

/// A CLI that was found on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredAgent {
    pub kind: AgentKind,
    /// Absolute path to the executable.
    pub program: PathBuf,
    /// First line of `--version`, when the CLI answered in time.
    pub version: Option<String>,
}

/// Looks for every [`AgentKind`] and returns the ones that are installed.
///
/// This runs one subprocess per candidate, so callers should treat it as
/// blocking work and keep it off the UI thread.
pub fn discover() -> Vec<DiscoveredAgent> {
    let directories = search_directories();

    AgentKind::ALL
        .iter()
        .filter_map(|&kind| {
            let program = locate(kind, &directories)?;
            let version = probe_version(&program);
            tracing::info!(
                agent = kind.display_name(),
                ?program,
                ?version,
                "found agent CLI"
            );
            Some(DiscoveredAgent {
                kind,
                program,
                version,
            })
        })
        .collect()
}

/// Runs `program --version` and returns its first non-empty line.
pub fn probe_version(program: &Path) -> Option<String> {
    let mut command = Command::new(program);
    command.arg("--version");

    let output = capture_stdout(command, VERSION_TIMEOUT)?;
    output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_owned)
}

/// Finds the executable for `kind`, preferring whatever `PATH` resolves to.
fn locate(kind: AgentKind, directories: &[PathBuf]) -> Option<PathBuf> {
    let candidate = directories
        .iter()
        .map(|directory| directory.join(kind.program()))
        .find(|path| is_executable(path));

    if candidate.is_some() {
        return candidate;
    }

    let home = home_directory()?;
    kind.extra_locations(&home)
        .into_iter()
        .find(|path| is_executable(path))
}

/// Every directory worth searching, most authoritative first, deduplicated.
fn search_directories() -> Vec<PathBuf> {
    let inherited = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .unwrap_or_default();

    let from_shell = login_shell_path()
        .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .unwrap_or_default();

    let mut seen = HashSet::new();
    inherited
        .into_iter()
        .chain(from_shell)
        .chain(well_known_bin_directories())
        .filter(|path| !path.as_os_str().is_empty())
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

/// Bin directories that are worth checking even when they are not on `PATH`.
fn well_known_bin_directories() -> Vec<PathBuf> {
    let mut directories = vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ];

    if let Some(home) = home_directory() {
        directories.extend(
            [
                ".local/bin",
                ".bun/bin",
                ".cargo/bin",
                ".volta/bin",
                ".npm-global/bin",
                ".yarn/bin",
                ".deno/bin",
            ]
            .into_iter()
            .map(|suffix| home.join(suffix)),
        );
    }

    directories
}

fn home_directory() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;

    std::fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_directories_are_unique() {
        let directories = search_directories();
        let unique: HashSet<_> = directories.iter().collect();
        assert_eq!(directories.len(), unique.len());
    }

    #[test]
    fn well_known_directories_are_absolute() {
        assert!(
            well_known_bin_directories()
                .iter()
                .all(|path| path.is_absolute())
        );
    }

    #[test]
    fn a_missing_program_is_not_executable() {
        assert!(!is_executable(Path::new("/nonexistent/pedro/agent")));
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_is_not_executable() {
        assert!(!is_executable(Path::new("/usr/bin")));
    }
}
