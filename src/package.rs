//! Package registry: the known first-party packages and how each installs.
//!
//! Only `install`/`uninstall`/`list` consult this. Dispatch (see main.rs) is
//! purely filesystem-based and never touches this enum — so an installed
//! package dispatches fine even if a future cdlvsm doesn't know its name.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use crate::download::{download, extract, latest_tag, need};
use crate::error::{fail, Result};
use crate::paths;

const CODE_REPO: &str = "codelovesme/code";
const EUGLENA_REPO: &str = "codelovesme/euglena-cli";

#[derive(Clone, Copy)]
pub enum Package {
    Code,
    Euglena,
}

impl Package {
    pub fn parse(name: &str) -> Option<Package> {
        match name {
            "code" => Some(Package::Code),
            "euglena" => Some(Package::Euglena),
            _ => None,
        }
    }

    pub fn install(self, opts: &InstallOpts) -> Result<()> {
        match self {
            Package::Code => install_code(opts),
            Package::Euglena => install_euglena(opts),
        }
    }
}

pub struct InstallOpts {
    pub tier: Tier,
    pub link: bool,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Tier {
    Sdk,
    Runtime,
}

impl Tier {
    fn as_str(self) -> &'static str {
        match self {
            Tier::Sdk => "sdk",
            Tier::Runtime => "runtime",
        }
    }
}

/// One package's release-fetch parameters. The binary inside the tarball, the
/// installed package dir, and the shim name are all `pkg`; `asset_base` is the
/// tarball/stage-dir prefix (which for `code` carries a tier infix, e.g.
/// `code-sdk`, and for `euglena` is just `euglena`).
struct ReleaseSpec<'a> {
    pkg: &'a str,
    repo: &'a str,
    asset_base: &'a str,
    tag: &'a str,
    label: Option<&'a str>,
    link: bool,
}

fn install_code(opts: &InstallOpts) -> Result<()> {
    let tier = opts.tier.as_str();
    let tag = resolve_tag("CDLVSM_CODE_VERSION", CODE_REPO)?;
    install_release(&ReleaseSpec {
        pkg: "code",
        repo: CODE_REPO,
        asset_base: &format!("code-{tier}"),
        tag: &tag,
        label: Some(tier),
        link: opts.link,
    })
}

fn install_euglena(opts: &InstallOpts) -> Result<()> {
    let tag = resolve_tag("CDLVSM_EUGLENA_VERSION", EUGLENA_REPO)?;
    install_release(&ReleaseSpec {
        pkg: "euglena",
        repo: EUGLENA_REPO,
        asset_base: "euglena",
        tag: &tag,
        label: None,
        link: opts.link,
    })
}

/// Resolve a release tag: a non-empty version pin from `env_var`, else the
/// repo's latest GitHub release.
fn resolve_tag(env_var: &str, repo: &str) -> Result<String> {
    match std::env::var(env_var) {
        Ok(v) if !v.is_empty() => Ok(v),
        _ => {
            eprintln!("Fetching latest release info...");
            latest_tag(repo)
        }
    }
}

/// Shared download → extract → install → symlink path for every package.
fn install_release(spec: &ReleaseSpec) -> Result<()> {
    need("curl")?;
    need("tar")?;

    let (os, arch) = platform();
    if os != "Linux" || arch != "x86_64" {
        return fail(format!(
            "prebuilt binaries are only available for Linux x86_64 (detected: {os} {arch}).\n\
             Build from source instead — see https://github.com/{}#building-from-source",
            spec.repo
        ));
    }

    let asset = format!("{}-{}-x86_64-linux.tar.gz", spec.asset_base, spec.tag);
    let url = format!(
        "https://github.com/{}/releases/download/{}/{asset}",
        spec.repo, spec.tag
    );

    let tmp = mktemp()?;
    let _guard = TmpGuard(tmp.clone());
    let tarball = tmp.join(&asset);
    eprintln!("Downloading {url}...");
    download(&url, &tarball)?;
    extract(&tarball, &tmp)?;

    // The tarball stages under an `<asset_base>-*` directory containing the
    // package binary (named `pkg`).
    let stage = find_stage(&tmp, &format!("{}-", spec.asset_base))?;
    let src_bin = stage.join(spec.pkg);
    if !src_bin.exists() {
        return fail(format!(
            "unexpected archive layout — no '{}' binary in {}",
            spec.pkg,
            stage.display()
        ));
    }

    let dest = paths::package_dir(spec.pkg).join(spec.tag);
    if dest.exists() {
        rm_rf(&dest);
    }
    fs::create_dir_all(&dest)
        .map_err(|e| crate::error::CdlvsmError(format!("mkdir {}: {e}", dest.display())))?;
    let dest_bin = dest.join(spec.pkg);
    fs::copy(&src_bin, &dest_bin)
        .map_err(|e| crate::error::CdlvsmError(format!("copy {} binary: {e}", spec.pkg)))?;
    // Don't trust the tarball to preserve the exec bit — exec() fails without it.
    make_executable(&dest_bin)?;

    // Repoint `current` -> <tag>.
    let current = paths::current_link(spec.pkg);
    let _ = fs::remove_file(&current);
    symlink(Path::new(spec.tag), &current)?;

    // Shims. cdlvsm-<pkg> always; bare `<pkg>` only with --link. For `code`
    // this opt-in avoids colliding with VS Code's own `code` CLI on Linux;
    // for other packages it's the same uniform rule (dispatch via
    // `cdlvsm <pkg> …` never needs the bare name).
    let bin = paths::bin_dir();
    fs::create_dir_all(&bin)
        .map_err(|e| crate::error::CdlvsmError(format!("mkdir {}: {e}", bin.display())))?;
    let target = paths::current_link(spec.pkg).join(spec.pkg);
    force_symlink(&target, &bin.join(format!("cdlvsm-{}", spec.pkg)))?;
    if spec.link {
        force_symlink(&target, &bin.join(spec.pkg))?;
    }

    eprintln!();
    match spec.label {
        Some(label) => eprintln!(
            "Installed {} ({label}) {} -> {}",
            spec.pkg,
            spec.tag,
            dest.display()
        ),
        None => eprintln!("Installed {} {} -> {}", spec.pkg, spec.tag, dest.display()),
    }
    eprintln!(
        "Shim: {}",
        bin.join(format!("cdlvsm-{}", spec.pkg)).display()
    );
    if spec.link {
        eprintln!("Linked: {}", bin.join(spec.pkg).display());
    }
    path_hint(&bin);
    Ok(())
}

pub fn uninstall(name: &str) -> Result<()> {
    let pkg_dir = paths::package_dir(name);
    if !pkg_dir.exists() {
        return fail(format!("'{name}' is not installed via cdlvsm."));
    }

    // Only remove a shim that is a symlink whose target resolves back inside
    // this package's own directory — never a same-named binary cdlvsm didn't
    // create (e.g. VS Code's real `code`). This is the single most important
    // safety property of the tool.
    let bin = paths::bin_dir();
    for shim in [format!("cdlvsm-{name}"), name.to_string()] {
        let candidate = bin.join(&shim);
        if let Ok(meta) = fs::symlink_metadata(&candidate) {
            if meta.file_type().is_symlink() {
                if let Ok(target) = fs::read_link(&candidate) {
                    let resolved = if target.is_absolute() {
                        target
                    } else {
                        bin.join(target)
                    };
                    // Canonicalize the package dir for a robust prefix check;
                    // fall back to the raw path if canonicalize fails.
                    let pkg_canon = fs::canonicalize(&pkg_dir).unwrap_or_else(|_| pkg_dir.clone());
                    let resolved_canon =
                        fs::canonicalize(&resolved).unwrap_or_else(|_| resolved.clone());
                    if resolved_canon.starts_with(&pkg_canon) {
                        let _ = fs::remove_file(&candidate);
                    }
                }
            }
        }
    }

    rm_rf(&pkg_dir);
    println!("Uninstalled {name}.");
    Ok(())
}

pub fn list() -> Result<()> {
    let root = paths::packages_root();
    let entries = match fs::read_dir(&root) {
        Ok(e) => e,
        Err(_) => {
            println!("No cdlvsm-managed packages installed.");
            return Ok(());
        }
    };

    let mut any = false;
    let mut rows: Vec<(String, String)> = Vec::new();
    for entry in entries.flatten() {
        let pkg = entry.file_name().to_string_lossy().into_owned();
        let current = paths::current_link(&pkg);
        // Tolerate corrupted state: skip a package dir with no/broken current.
        if let Ok(target) = fs::read_link(&current) {
            let version = target.to_string_lossy().into_owned();
            rows.push((pkg, version));
            any = true;
        }
    }

    if !any {
        println!("No cdlvsm-managed packages installed.");
        return Ok(());
    }

    rows.sort();
    let bin = paths::bin_dir();
    for (pkg, version) in rows {
        println!(
            "{pkg}  {version}  (shim: {})",
            bin.join(format!("cdlvsm-{pkg}")).display()
        );
    }
    Ok(())
}

// --- small platform helpers -------------------------------------------------

fn platform() -> (String, String) {
    let os = run_capture("uname", &["-s"]).unwrap_or_default();
    let arch = run_capture("uname", &["-m"]).unwrap_or_default();
    (os, arch)
}

fn run_capture(cmd: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(cmd).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn mktemp() -> Result<std::path::PathBuf> {
    let s = run_capture("mktemp", &["-d"])
        .ok_or_else(|| crate::error::CdlvsmError("mktemp failed".into()))?;
    Ok(std::path::PathBuf::from(s))
}

struct TmpGuard(std::path::PathBuf);
impl Drop for TmpGuard {
    fn drop(&mut self) {
        rm_rf(&self.0);
    }
}

fn find_stage(tmp: &Path, prefix: &str) -> Result<std::path::PathBuf> {
    let entries = fs::read_dir(tmp)
        .map_err(|e| crate::error::CdlvsmError(format!("read {}: {e}", tmp.display())))?;
    for entry in entries.flatten() {
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(prefix) {
                return Ok(entry.path());
            }
        }
    }
    fail(format!(
        "unexpected archive layout — no {prefix}* directory found."
    ))
}

fn make_executable(p: &Path) -> Result<()> {
    let mut perms = fs::metadata(p)
        .map_err(|e| crate::error::CdlvsmError(format!("stat {}: {e}", p.display())))?
        .permissions();
    perms.set_mode(perms.mode() | 0o755);
    fs::set_permissions(p, perms)
        .map_err(|e| crate::error::CdlvsmError(format!("chmod {}: {e}", p.display())))
}

fn symlink(target: &Path, link: &Path) -> Result<()> {
    if let Some(parent) = link.parent() {
        let _ = fs::create_dir_all(parent);
    }
    std::os::unix::fs::symlink(target, link)
        .map_err(|e| crate::error::CdlvsmError(format!("symlink {}: {e}", link.display())))
}

fn force_symlink(target: &Path, link: &Path) -> Result<()> {
    let _ = fs::remove_file(link);
    symlink(target, link)
}

fn rm_rf(p: &Path) {
    let _ = fs::remove_dir_all(p);
    let _ = fs::remove_file(p);
}

fn path_hint(bin: &Path) {
    if let Some(path) = std::env::var_os("PATH") {
        for entry in std::env::split_paths(&path) {
            if entry == bin {
                return;
            }
        }
    }
    eprintln!();
    eprintln!(
        "Note: {} is not on your PATH. Add this to your shell profile:",
        bin.display()
    );
    eprintln!("  export PATH=\"{}:$PATH\"", bin.display());
}
