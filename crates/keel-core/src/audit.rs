//! Deterministic change-set identity used by commit and PR audit gates.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuditTarget {
    pub scope: String,
    pub mode: String,
    pub files: Vec<String>,
}

pub fn target_for_command(root: &Path, command: Option<&str>) -> Option<AuditTarget> {
    let tokens = shell_words(command?);
    if is_git_commit(&tokens) {
        return target_for_commit(root);
    }
    if is_pr_command(&tokens) {
        return target_for_pr(root, pr_base(root, &tokens).as_deref());
    }
    None
}

/// Exact staged change set that a `git commit` will record.
pub fn target_for_commit(root: &Path) -> Option<AuditTarget> {
    target_from_patch(git(
        root,
        &[
            "diff",
            "--cached",
            "--binary",
            "--no-ext-diff",
            "--no-color",
        ],
    )?)
}

/// Exact branch change set that a PR would present against its base branch.
pub fn target_for_pr(root: &Path, base: Option<&str>) -> Option<AuditTarget> {
    let base = match base {
        Some(base) => base.to_string(),
        None => pr_base(root, &[])?,
    };
    target_from_patch(git(
        root,
        &[
            "diff",
            &format!("{base}...HEAD"),
            "--binary",
            "--no-ext-diff",
            "--no-color",
        ],
    )?)
}

fn target_from_patch(patch: String) -> Option<AuditTarget> {
    let mut files = patch
        .lines()
        .filter_map(|line| {
            line.strip_prefix("+++ b/")
                .or_else(|| line.strip_prefix("--- a/"))
        })
        .filter(|path| *path != "/dev/null")
        .map(str::to_string)
        .collect::<Vec<_>>();
    files.sort();
    files.dedup();

    let mode = classify(&files, &patch);
    let mut hasher = Sha256::new();
    hasher.update(b"keel-audit-scope-v1\n");
    hasher.update(patch.as_bytes());
    let scope = format!("sha256:{:x}", hasher.finalize());
    Some(AuditTarget { scope, mode, files })
}

fn classify(files: &[String], patch: &str) -> String {
    let changed_lines = patch
        .lines()
        .filter(|line| {
            (line.starts_with('+') || line.starts_with('-'))
                && !line.starts_with("+++")
                && !line.starts_with("---")
        })
        .count();
    let high_risk = files.iter().any(|file| {
        let lower = file.to_ascii_lowercase();
        [
            "auth",
            "security",
            "payment",
            "billing",
            "migration",
            "permission",
        ]
        .iter()
        .any(|term| lower.contains(term))
    });
    if high_risk || files.len() > 30 || changed_lines > 1000 {
        "exhaustive".into()
    } else if files.len() > 10
        || changed_lines > 400
        || files.iter().any(|file| {
            let lower = file.to_ascii_lowercase();
            [
                "provider",
                "notifier",
                "viewmodel",
                "repository",
                "service",
                "api",
                "model",
            ]
            .iter()
            .any(|term| lower.contains(term))
        })
    {
        "extended".into()
    } else {
        "focused".into()
    }
}

fn is_git_commit(tokens: &[String]) -> bool {
    tokens.first().is_some_and(|token| token == "git")
        && tokens.iter().skip(1).any(|token| token == "commit")
}

fn is_pr_command(tokens: &[String]) -> bool {
    tokens.first().is_some_and(|token| token == "gh")
        && tokens.get(1).is_some_and(|token| token == "pr")
        && tokens
            .get(2)
            .is_some_and(|token| token == "create" || token == "edit")
}

fn pr_base(root: &Path, tokens: &[String]) -> Option<String> {
    if let Some(index) = tokens.iter().position(|token| token == "--base")
        && let Some(base) = tokens.get(index + 1)
    {
        return Some(base.clone());
    }
    let head = git(
        root,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    )?;
    head.strip_prefix("origin/").map(str::to_string)
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn shell_words(command: &str) -> Vec<String> {
    command
        .split_whitespace()
        .map(|token| token.trim_matches(['\'', '"']).to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_local_change_is_focused() {
        assert_eq!(classify(&["lib/invoice.dart".into()], "+a\n-b"), "focused");
    }

    #[test]
    fn state_change_is_extended() {
        assert_eq!(
            classify(&["lib/current_route_provider.dart".into()], "+a"),
            "extended"
        );
    }

    #[test]
    fn sensitive_change_is_exhaustive() {
        assert_eq!(
            classify(&["lib/auth/session.dart".into()], "+a"),
            "exhaustive"
        );
    }
}
