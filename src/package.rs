//! Package registry: the known first-party packages and how each installs.
//!
//! Only `install`/`uninstall`/`list` consult this. Dispatch (see main.rs) is
//! purely filesystem-based and never touches this enum — so an installed
//! package dispatches fine even if a future cdlvsm doesn't know its name.

use std::fs;
use std::path::Path;

use crate::download::{
    download, extract, find_stage, latest_tag, make_executable, mktemp, need, platform,
    release_assets, TmpGuard,
};
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

    /// Default install settings for a package whose `.cdlvsm` metadata is
    /// missing (installed by an older cdlvsm): the SDK tier, no bare link.
    /// `upgrade` falls back to these with a warning rather than failing.
    fn default_meta(self) -> InstallMeta {
        match self {
            Package::Code => InstallMeta {
                pkg: "code".into(),
                repo: CODE_REPO.into(),
                asset_base: "code-sdk".into(),
                env_var: "CDLVSM_CODE_VERSION".into(),
                label: Some("sdk".into()),
                link: false,
            },
            Package::Euglena => InstallMeta {
                pkg: "euglena".into(),
                repo: EUGLENA_REPO.into(),
                asset_base: "euglena".into(),
                env_var: "CDLVSM_EUGLENA_VERSION".into(),
                label: None,
                link: false,
            },
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

/// How a package was installed, recorded in the `.cdlvsm` metadata file so
/// `upgrade` can reuse the exact settings (a `--runtime` install must not be
/// silently reset to SDK). Written by `install_release` on every successful
/// install/upgrade; read by `upgrade`.
#[derive(Clone)]
struct InstallMeta {
    pkg: String,
    repo: String,
    asset_base: String,
    env_var: String,
    label: Option<String>,
    link: bool,
}

impl InstallMeta {
    fn from_spec(spec: &ReleaseSpec) -> InstallMeta {
        InstallMeta {
            pkg: spec.pkg.to_string(),
            repo: spec.repo.to_string(),
            asset_base: spec.asset_base.to_string(),
            env_var: spec.env_var.to_string(),
            label: spec.label.map(str::to_string),
            link: spec.link,
        }
    }
}

/// Read the `.cdlvsm` metadata file for `pkg`. `Ok(None)` when the file is
/// absent (an install by an older cdlvsm); `Err` only on a genuinely
/// unreadable file.
fn read_meta(pkg: &str) -> Result<Option<InstallMeta>> {
    let path = paths::metadata_path(pkg);
    let body = match fs::read_to_string(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return fail(format!(
                "could not read install metadata {}: {e}",
                path.display()
            ))
        }
    };

    let mut meta: Option<InstallMeta> = None;
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = match line.split_once('=') {
            Some(kv) => kv,
            None => continue, // tolerate malformed lines
        };
        match key {
            "pkg" => {
                meta = Some(InstallMeta {
                    pkg: value.to_string(),
                    repo: String::new(),
                    asset_base: String::new(),
                    env_var: String::new(),
                    label: None,
                    link: false,
                });
            }
            "repo" => {
                if let Some(m) = meta.as_mut() {
                    m.repo = value.to_string();
                }
            }
            "asset_base" => {
                if let Some(m) = meta.as_mut() {
                    m.asset_base = value.to_string();
                }
            }
            "env_var" => {
                if let Some(m) = meta.as_mut() {
                    m.env_var = value.to_string();
                }
            }
            "label" => {
                if !value.is_empty() {
                    if let Some(m) = meta.as_mut() {
                        m.label = Some(value.to_string());
                    }
                }
            }
            "link" => {
                if let Some(m) = meta.as_mut() {
                    m.link = value == "1";
                }
            }
            _ => {} // unknown keys are ignored (forward compatibility)
        }
    }

    match meta {
        Some(m) if !m.pkg.is_empty() && !m.repo.is_empty() && !m.asset_base.is_empty() => {
            Ok(Some(m))
        }
        _ => Ok(None), // present but unusable — treat as absent
    }
}

/// Write the `.cdlvsm` metadata file for `meta.pkg`.
fn write_meta(meta: &InstallMeta) -> Result<()> {
    let path = paths::metadata_path(&meta.pkg);
    let mut body = format!(
        "pkg={}\nrepo={}\nasset_base={}\nenv_var={}\n",
        meta.pkg, meta.repo, meta.asset_base, meta.env_var
    );
    match &meta.label {
        Some(label) => body.push_str(&format!("label={label}\n")),
        None => body.push_str("label=\n"),
    }
    body.push_str(&format!("link={}\n", if meta.link { 1 } else { 0 }));
    fs::write(&path, body)
        .map_err(|e| crate::error::CdlvsmError(format!("write {}: {e}", path.display())))
}

/// One package's release-fetch parameters. The binary inside the tarball, the
/// installed package dir, and the shim name are all `pkg`; `asset_base` is the
/// tarball/stage-dir prefix (which for `code` carries a tier infix, e.g.
/// `code-sdk`, and for `euglena` is just `euglena`). `env_var` is the
/// version-pin env var recorded in the metadata file; `verb`/`old` shape the
/// final message ("Installed …" vs "Upgraded … v0.2.0 -> v0.3.0").
struct ReleaseSpec<'a> {
    pkg: &'a str,
    repo: &'a str,
    asset_base: &'a str,
    tag: &'a str,
    label: Option<&'a str>,
    link: bool,
    env_var: &'a str,
    verb: &'a str,
    old: Option<&'a str>,
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
        env_var: "CDLVSM_CODE_VERSION",
        verb: "Installed",
        old: None,
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
        env_var: "CDLVSM_EUGLENA_VERSION",
        verb: "Installed",
        old: None,
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

    let (asset, tiered) = resolve_asset(spec)?;
    let url = format!(
        "https://github.com/{}/releases/download/{}/{asset}",
        spec.repo, spec.tag
    );

    // When the tier-specific asset doesn't exist for this release, the single
    // untiered build is what gets installed — say so, and don't label the
    // result with a tier it doesn't have.
    let label = if tiered { spec.label } else { None };
    if !tiered {
        if let Some(l) = spec.label {
            eprintln!(
                "note: {} {} has no separate '{l}' build; using the single {} release asset.",
                spec.pkg, spec.tag, spec.pkg
            );
        }
    }

    let tmp = mktemp()?;
    let _guard = TmpGuard(tmp.clone());
    let tarball = tmp.join(&asset);
    eprintln!("Downloading {url}...");
    download(&url, &tarball)?;
    extract(&tarball, &tmp)?;

    // The tarball stages under a directory named after the asset (minus the
    // `.tar.gz`) containing the package binary (named `pkg`).
    let stage = find_stage(&tmp, asset.trim_end_matches(".tar.gz"), spec.pkg)?;
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

    // Record how this package was *requested*, not how `resolve_asset` fell
    // back this time, so `upgrade` keeps asking for the same tier — if a later
    // release brings tiered builds back, it picks them up again.
    write_meta(&InstallMeta::from_spec(spec))?;

    eprintln!();
    match (spec.verb, spec.old) {
        ("Upgraded", Some(old)) => match label {
            Some(label) => eprintln!("Upgraded {} ({label}) {old} -> {}", spec.pkg, spec.tag),
            None => eprintln!("Upgraded {} {old} -> {}", spec.pkg, spec.tag),
        },
        ("Upgraded", None) => match label {
            Some(label) => eprintln!("Upgraded {} ({label}) -> {}", spec.pkg, spec.tag),
            None => eprintln!("Upgraded {} -> {}", spec.pkg, spec.tag),
        },
        _ => match label {
            Some(label) => eprintln!(
                "Installed {} ({label}) {} -> {}",
                spec.pkg,
                spec.tag,
                dest.display()
            ),
            None => eprintln!("Installed {} {} -> {}", spec.pkg, spec.tag, dest.display()),
        },
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

/// Pick the release asset to download.
///
/// Upstream asset naming isn't frozen: `code` shipped per-tier tarballs
/// (`code-sdk-…`, `code-runtime-…`) through v0.4.1 and a single untiered
/// `code-…` tarball from v0.5.0 on. So rather than trusting one constructed
/// name, ask the release what it actually carries and take the first candidate
/// it offers — tier-specific name first, then the plain `<pkg>` name.
///
/// Returns the asset file name and whether it was the tier-specific one.
fn resolve_asset(spec: &ReleaseSpec) -> Result<(String, bool)> {
    let preferred = format!("{}-{}-x86_64-linux.tar.gz", spec.asset_base, spec.tag);
    let plain = format!("{}-{}-x86_64-linux.tar.gz", spec.pkg, spec.tag);

    // No listing (offline, rate-limited, private repo): assume the
    // conventional name and let `download` report the real failure.
    let assets = match release_assets(spec.repo, spec.tag) {
        Some(a) => a,
        None => return Ok((preferred, true)),
    };

    if assets.contains(&preferred) {
        return Ok((preferred, true));
    }
    if plain != preferred && assets.contains(&plain) {
        return Ok((plain, false));
    }

    let also = if plain == preferred {
        String::new()
    } else {
        format!(" or '{plain}'")
    };
    fail(format!(
        "release {} of {} has no asset named '{preferred}'{also}.\n\
         Available assets: {}",
        spec.tag,
        spec.repo,
        assets.join(", ")
    ))
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

/// Upgrade one package to its latest (or version-pin env var) release,
/// reusing the recorded install settings from `.cdlvsm`. If the resolved tag
/// equals the currently active version, the package is skipped with an
/// "already up to date" note — no download.
pub fn upgrade(name: &str) -> Result<()> {
    let pkg_dir = paths::package_dir(name);
    if !pkg_dir.exists() {
        if Package::parse(name).is_some() {
            return fail(format!(
                "'{name}' is not installed — run `cdlvsm install {name}` first."
            ));
        }
        return fail(format!("unknown package '{name}'"));
    }

    // Current version from the `current` symlink. A broken/missing symlink is
    // tolerated: the upgrade proceeds with `old = None` and self-heals.
    let current = match fs::read_link(paths::current_link(name)) {
        Ok(t) => Some(t.to_string_lossy().into_owned()),
        Err(_) => None,
    };

    // Install settings: recorded metadata, else (for packages this cdlvsm
    // knows) the enum defaults with a warning.
    let meta = match read_meta(name)? {
        Some(m) => m,
        None => match Package::parse(name) {
            Some(pkg) => {
                eprintln!(
                    "warning: no install metadata for '{name}' (installed by an older cdlvsm); \
                     assuming default settings"
                );
                pkg.default_meta()
            }
            None => {
                return fail(format!(
                    "no install metadata for '{name}' — cannot determine how to upgrade"
                ))
            }
        },
    };

    let tag = resolve_tag(&meta.env_var, &meta.repo)?;
    if let Some(cur) = &current {
        if *cur == tag {
            println!("{name} is already up to date ({tag})");
            return Ok(());
        }
    }

    install_release(&ReleaseSpec {
        pkg: &meta.pkg,
        repo: &meta.repo,
        asset_base: &meta.asset_base,
        tag: &tag,
        label: meta.label.as_deref(),
        link: meta.link,
        env_var: &meta.env_var,
        verb: "Upgraded",
        old: current.as_deref(),
    })
}

/// Upgrade every installed package. Per-package errors are reported and
/// skipped; the command fails (exit 1) if any package failed.
pub fn upgrade_all() -> Result<()> {
    let root = paths::packages_root();
    let entries = match fs::read_dir(&root) {
        Ok(e) => e,
        Err(_) => {
            println!("No cdlvsm-managed packages installed.");
            return Ok(());
        }
    };

    let mut names: Vec<String> = entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();

    if names.is_empty() {
        println!("No cdlvsm-managed packages installed.");
        return Ok(());
    }

    let mut failed = 0;
    for name in &names {
        if let Err(e) = upgrade(name) {
            eprintln!("error: {name}: {}", e.0);
            failed += 1;
        }
    }
    if failed > 0 {
        return fail(format!("{failed} package(s) failed to upgrade"));
    }
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

// --- small filesystem helpers ------------------------------------------------

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
