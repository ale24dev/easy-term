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
    resolve_shell_path_using(&shell)
}

/// Split out from `resolve_login_shell_path` so tests can point it at a
/// specific shell binary instead of racing on the process-global `$SHELL`.
fn resolve_shell_path_using(shell: &str) -> Option<String> {
    let output = Command::new(shell)
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
    merge_overrides(RESOLVED_PATH.get().map(String::as_str), extra)
}

/// Split out from `overrides` so tests can exercise the merge logic without
/// depending on `RESOLVED_PATH`, a `OnceLock` that (correctly) can only be
/// set once per process — and is shared by every test in this binary.
fn merge_overrides(
    base_path: Option<&str>,
    extra: &HashMap<String, String>,
) -> Vec<(String, String)> {
    let mut result = Vec::new();
    if let Some(path) = base_path {
        result.push(("PATH".to_string(), path.to_string()));
    }
    for (key, value) in extra {
        result.push((key.clone(), value.clone()));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_shell_path_using_sh_returns_a_nonempty_path() {
        let path = resolve_shell_path_using("/bin/sh").expect("/bin/sh must be resolvable here");
        assert!(!path.trim().is_empty());
    }

    #[test]
    fn resolve_shell_path_using_bash_returns_a_nonempty_path() {
        let path = resolve_shell_path_using("/usr/bin/bash").expect("bash must be resolvable here");
        assert!(!path.trim().is_empty());
    }

    #[test]
    fn resolve_shell_path_using_a_nonexistent_shell_returns_none() {
        assert_eq!(
            resolve_shell_path_using("/definitely/not/a/real/shell-binary"),
            None
        );
    }

    #[test]
    fn merge_overrides_with_no_base_path_only_includes_extra_vars() {
        let mut extra = HashMap::new();
        extra.insert("NODE_ENV".to_string(), "development".to_string());

        let result = merge_overrides(None, &extra);

        assert_eq!(
            result,
            vec![("NODE_ENV".to_string(), "development".to_string())]
        );
    }

    #[test]
    fn merge_overrides_puts_the_base_path_first_then_extra_vars() {
        let mut extra = HashMap::new();
        extra.insert("FOO".to_string(), "bar".to_string());

        let result = merge_overrides(Some("/usr/bin:/bin"), &extra);

        assert_eq!(result[0], ("PATH".to_string(), "/usr/bin:/bin".to_string()));
        assert!(result.contains(&("FOO".to_string(), "bar".to_string())));
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn merge_overrides_with_nothing_at_all_is_empty() {
        assert!(merge_overrides(None, &HashMap::new()).is_empty());
    }
}
