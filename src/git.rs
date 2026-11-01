use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::Command;

/// Run git and return trimmed stdout. Errors carry the command and stderr.
pub fn run_git(args: &[&str], cwd: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .with_context(|| format!("running git {}", args.join(" ")))?;

    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Run git and return `None` when it exits non-zero, for probes such as
/// `git describe` on a repository with no tags.
pub fn try_git(args: &[&str], cwd: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

pub fn fetch_tags(cwd: &Path) -> Result<()> {
    run_git(&["fetch", "--tags", "--force", "origin"], cwd)?;
    Ok(())
}
