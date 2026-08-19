//! Filesystem layout for cdlvsm-managed installs.
//!
//! Everything hangs off `$PREFIX` (default `$HOME/.local`):
//!
//!   $PREFIX/share/cdlvsm/packages/<pkg>/<version>/   extracted package
//!   $PREFIX/share/cdlvsm/packages/<pkg>/current      symlink -> active version
//!   $PREFIX/bin/cdlvsm-<pkg>                          shim -> current/<pkg>
//!   $PREFIX/bin/<pkg>                                 bare command (only with --link)

use std::path::PathBuf;

pub fn prefix() -> PathBuf {
    if let Some(p) = std::env::var_os("PREFIX") {
        return PathBuf::from(p);
    }
    // Fall back to $HOME/.local. If HOME itself is unset (extremely unusual),
    // use ".local" relative to cwd rather than panicking — a clean best-effort.
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join(".local")
}

pub fn bin_dir() -> PathBuf {
    prefix().join("bin")
}

pub fn packages_root() -> PathBuf {
    prefix().join("share").join("cdlvsm").join("packages")
}

pub fn package_dir(pkg: &str) -> PathBuf {
    packages_root().join(pkg)
}

pub fn current_link(pkg: &str) -> PathBuf {
    package_dir(pkg).join("current")
}

/// The binary cdlvsm dispatches to: `.../packages/<pkg>/current/<pkg>`.
/// Binary name == package name for every package cdlvsm manages.
pub fn dispatch_target(pkg: &str) -> PathBuf {
    current_link(pkg).join(pkg)
}
