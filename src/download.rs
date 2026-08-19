//! Download/extract seam.
//!
//! Shells out to `curl`/`tar` rather than pulling in HTTP/archive crates —
//! keeps the dependency set empty and mirrors the proven shell prototype 1:1.
//! Kept behind these two functions so the implementation can be swapped (or
//! faked in tests) without touching install logic.

use std::path::Path;
use std::process::Command;

use crate::error::{fail, Result};

/// Assert an external tool is on PATH, or fail cleanly.
pub fn need(tool: &str) -> Result<()> {
    let found = Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {tool} >/dev/null 2>&1", tool = tool))
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !found {
        return fail(format!("'{tool}' is required but not found on PATH."));
    }
    Ok(())
}

/// Fetch the latest release tag for `owner/repo` via the GitHub API.
///
/// Deliberately the same grep/sed extraction the shell prototype used — not a
/// JSON dependency. Good enough for GitHub's stable `"tag_name": "..."` field.
pub fn latest_tag(repo: &str) -> Result<String> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let out = Command::new("curl")
        .args(["-fsSL", &url])
        .output()
        .map_err(|e| crate::error::CdlvsmError(format!("failed to run curl: {e}")))?;
    if !out.status.success() {
        return fail(format!("could not fetch latest release info for {repo}"));
    }
    let body = String::from_utf8_lossy(&out.stdout);
    for line in body.lines() {
        if let Some(idx) = line.find("\"tag_name\"") {
            // ..."tag_name": "v0.3.0",...
            let rest = &line[idx..];
            if let Some(colon) = rest.find(':') {
                let after = &rest[colon + 1..];
                if let Some(start) = after.find('"') {
                    let tail = &after[start + 1..];
                    if let Some(end) = tail.find('"') {
                        let tag = tail[..end].trim().to_string();
                        if !tag.is_empty() {
                            return Ok(tag);
                        }
                    }
                }
            }
        }
    }
    fail(format!(
        "could not determine the latest release version for {repo}"
    ))
}

pub fn download(url: &str, dest: &Path) -> Result<()> {
    let status = Command::new("curl")
        .args(["-fsSL", url, "-o"])
        .arg(dest)
        .status()
        .map_err(|e| crate::error::CdlvsmError(format!("failed to run curl: {e}")))?;
    if !status.success() {
        return fail(format!("download failed: {url}"));
    }
    Ok(())
}

pub fn extract(tarball: &Path, into: &Path) -> Result<()> {
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(tarball)
        .arg("-C")
        .arg(into)
        .status()
        .map_err(|e| crate::error::CdlvsmError(format!("failed to run tar: {e}")))?;
    if !status.success() {
        return fail(format!("extraction failed: {}", tarball.display()));
    }
    Ok(())
}
