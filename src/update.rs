//! Self-update: replace the running `cdlvsm` binary with the latest (or
//! pinned) release from GitHub, in place.
//!
//! Unlike a package (see package.rs), cdlvsm itself isn't installed under
//! `packages/<pkg>/<version>/current` — `install.sh` copies the binary
//! straight to `$PREFIX/bin/cdlvsm`. So `update` downloads the new binary and
//! renames it over the running executable's own path: on Linux that's safe
//! even while it's executing — the kernel keeps the old (now-unlinked) inode
//! alive via this process's mapped pages, and the new file takes effect on
//! the next invocation. No shell restart needed.

use std::fs;

use crate::download::{
    download, extract, find_stage, latest_tag, make_executable, mktemp, need, platform,
    release_assets, TmpGuard,
};
use crate::error::{fail, Result};

const REPO: &str = "codelovesme/cdlvsm-cli";
const ENV_VAR: &str = "CDLVSM_CLI_VERSION";

pub fn update() -> Result<()> {
    need("curl")?;
    need("tar")?;

    let (os, arch) = platform();
    if os != "Linux" || arch != "x86_64" {
        return fail(format!(
            "prebuilt binaries are only available for Linux x86_64 (detected: {os} {arch}).\n\
             Build from source instead — see https://github.com/{REPO}#building-from-source"
        ));
    }

    let current_version = concat!("v", env!("CARGO_PKG_VERSION"));
    let tag = match std::env::var(ENV_VAR) {
        Ok(v) if !v.is_empty() => v,
        _ => {
            eprintln!("Fetching latest release info...");
            latest_tag(REPO)?
        }
    };

    if tag == current_version {
        println!("cdlvsm is already up to date ({tag})");
        return Ok(());
    }

    let asset = format!("cdlvsm-{tag}-x86_64-linux.tar.gz");
    if let Some(assets) = release_assets(REPO, &tag) {
        if !assets.contains(&asset) {
            return fail(format!(
                "release {tag} of {REPO} has no asset named '{asset}'.\nAvailable assets: {}",
                assets.join(", ")
            ));
        }
    }

    let url = format!("https://github.com/{REPO}/releases/download/{tag}/{asset}");

    let tmp = mktemp()?;
    let _guard = TmpGuard(tmp.clone());
    let tarball = tmp.join(&asset);
    eprintln!("Downloading {url}...");
    download(&url, &tarball)?;
    extract(&tarball, &tmp)?;

    let stage = find_stage(&tmp, &format!("cdlvsm-{tag}"), "cdlvsm")?;
    let new_bin = stage.join("cdlvsm");
    if !new_bin.exists() {
        return fail(format!(
            "unexpected archive layout — no 'cdlvsm' binary in {}",
            stage.display()
        ));
    }
    make_executable(&new_bin)?;

    let current_exe = std::env::current_exe().map_err(|e| {
        crate::error::CdlvsmError(format!("could not determine own executable path: {e}"))
    })?;
    let parent = current_exe.parent().ok_or_else(|| {
        crate::error::CdlvsmError(format!("no parent directory for {}", current_exe.display()))
    })?;

    // Stage the new binary next to the old one, then rename over it: the
    // rename is atomic and same-filesystem, and safe to do to a file that's
    // currently executing.
    let staged = parent.join(".cdlvsm-update.tmp");
    fs::copy(&new_bin, &staged)
        .map_err(|e| crate::error::CdlvsmError(format!("copy new binary into place: {e}")))?;
    make_executable(&staged)?;
    fs::rename(&staged, &current_exe).map_err(|e| {
        crate::error::CdlvsmError(format!("replace {}: {e}", current_exe.display()))
    })?;

    eprintln!();
    eprintln!("Updated cdlvsm {current_version} -> {tag}");
    Ok(())
}
