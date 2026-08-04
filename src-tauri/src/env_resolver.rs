//! Resolves the user's real PATH from a login shell and caches it.
//!
//! Apps launched from the macOS menu bar / Finder inherit a minimal PATH
//! (`/usr/bin:/bin:...`), not the one set up in `.zshrc`/`.bashrc` by
//! nvm/fnm/asdf/homebrew. Without this, spawned dev servers fail with
//! "command not found" for tools the user runs from their terminal every day.

use crate::error_logger::{log_error, Level, Source};
use std::collections::HashMap;
use std::env;
use std::process::Command;
use std::sync::OnceLock;

const MODULE: &str = "env_resolver";

static RESOLVED_PATH: OnceLock<String> = OnceLock::new();

/// Resolves and caches the login shell's PATH. Call once, early in `setup()`.
pub fn init() {
    let path = resolve_login_shell_path().unwrap_or_else(|| {
        log_error(
            Level::Warn,
            Source::Backend,
            MODULE,
            "ENV_SHELL_RESOLVE_FAILED",
            "No se pudo resolver el PATH del login shell; se usará el PATH del proceso actual",
            None,
            None,
        );
        env::var("PATH").unwrap_or_default()
    });

    if path.trim().is_empty() {
        log_error(
            Level::Warn,
            Source::Backend,
            MODULE,
            "ENV_PATH_EMPTY",
            "El PATH resuelto está vacío",
            None,
            None,
        );
    }

    let _ = RESOLVED_PATH.set(path);
}

fn resolve_login_shell_path() -> Option<String> {
    let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let output = Command::new(&shell)
        .arg("-lc")
        .arg("printf %s \"$PATH\"")
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let path = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if path.is_empty() {
        None
    } else {
        Some(path)
    }
}

/// Environment variable overrides spawned dev-server processes should apply
/// on top of the inherited process environment: the resolved login-shell
/// PATH, plus any project-specific variables.
///
/// `portable_pty::CommandBuilder` snapshots the current process environment
/// automatically, so callers only need to apply these overrides via `.env()`
/// rather than reconstructing the full environment.
pub fn overrides(extra: &HashMap<String, String>) -> Vec<(String, String)> {
    let mut result = Vec::new();
    if let Some(path) = RESOLVED_PATH.get() {
        result.push(("PATH".to_string(), path.clone()));
    }
    for (key, value) in extra {
        result.push((key.clone(), value.clone()));
    }
    result
}
