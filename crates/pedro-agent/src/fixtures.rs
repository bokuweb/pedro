//! A stand-in for an agent CLI, for tests across this workspace.
//!
//! Everything about a run except the CLI itself is worth testing without one:
//! whether deltas are stitched into an answer, whether a refusal is told apart
//! from a crash, whether a stored answer carries the sources it named. A shell
//! script that prints recorded JSONL covers all of it and needs no credentials.

use std::path::{Path, PathBuf};

/// Writes an executable script that prints `script` and exits, and returns its
/// path. Reusing a `name` overwrites the previous one.
///
/// Print recorded JSONL with `printf '%s\n'` rather than `echo`: `/bin/sh` on
/// macOS expands backslash escapes in `echo`, so a `\n` inside a recorded
/// string becomes a real newline and splits one event across two lines.
pub fn fake_cli(name: &str, script: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("pedro-fake-agent-{name}"));
    std::fs::create_dir_all(&directory).expect("a writable temp directory");

    let path = directory.join("agent");
    std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).expect("a writable script");
    make_executable(&path);

    path
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .expect("permissions can be set");
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) {}
