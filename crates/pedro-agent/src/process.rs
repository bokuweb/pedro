//! Small helpers for running short-lived probe commands.
//!
//! Everything here is expected to finish in milliseconds. A probe that hangs
//! must never block discovery, so every call is bounded by a timeout and the
//! child is killed when it expires.

use std::io::Read as _;
use std::process::{Command, Stdio};
use std::time::Duration;

use wait_timeout::ChildExt as _;

/// Runs `command` and returns its stdout, or `None` if it fails, is killed by
/// the timeout, or exits with a non-zero status.
pub fn capture_stdout(mut command: Command, timeout: Duration) -> Option<String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        // Draining two pipes from one thread can deadlock, and no probe we run
        // needs stderr, so discard it.
        .stderr(Stdio::null());

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            tracing::debug!(?err, "failed to spawn probe command");
            return None;
        }
    };

    // Read on a worker thread: a child that writes more than the pipe buffer
    // would otherwise block forever waiting for us to drain it.
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });

    let status = match child.wait_timeout(timeout) {
        Ok(Some(status)) => status,
        Ok(None) => {
            tracing::debug!("probe command timed out, killing it");
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        Err(err) => {
            tracing::debug!(?err, "failed to wait for probe command");
            return None;
        }
    };

    let output = reader.join().ok()?;
    if !status.success() {
        return None;
    }

    Some(String::from_utf8_lossy(&output).into_owned())
}
