//! Integration tests for the `cdlvsm` CLI.
//!
//! Two groups:
//!   - Offline tests (error/usage/dispatch-resolution paths) — always run, no
//!     network. These include the uninstall-safety regression, exercised with
//!     a hand-built fake package tree so it needs no download.
//!   - Network tests that really install `code` and `euglena` from GitHub
//!     Releases, gated behind `CDLVSM_NETWORK_TESTS=1` so CI/offline runs stay
//!     fast and hermetic.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_cdlvsm")
}

fn tmp_prefix(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cdlvsm_it_{}_{}", std::process::id(), tag));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run(prefix: &Path, args: &[&str]) -> Out {
    let out = Command::new(bin())
        .args(args)
        .env("PREFIX", prefix)
        .env_remove("CDLVSM_CODE_VERSION")
        .env_remove("CDLVSM_EUGLENA_VERSION")
        .output()
        .unwrap();
    Out {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// Like `run`, but with extra env vars set (e.g. a version pin) — used by the
/// offline upgrade tests to short-circuit the network `latest_tag` fetch.
fn run_env(prefix: &Path, args: &[&str], extra_env: &[(&str, &str)]) -> Out {
    let mut cmd = Command::new(bin());
    cmd.args(args)
        .env("PREFIX", prefix)
        .env_remove("CDLVSM_CODE_VERSION")
        .env_remove("CDLVSM_EUGLENA_VERSION");
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    Out {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

// --- offline: usage / error contract ---------------------------------------

#[test]
fn no_args_prints_usage_exit_1() {
    let p = tmp_prefix("noargs");
    let o = run(&p, &[]);
    assert_eq!(o.code, 1);
    assert!(o.stdout.contains("Usage:"), "stdout: {}", o.stdout);
}

#[test]
fn version_exit_0() {
    let p = tmp_prefix("version");
    let o = run(&p, &["--version"]);
    assert_eq!(o.code, 0);
    assert!(o.stdout.contains("cdlvsm"), "stdout: {}", o.stdout);
}

#[test]
fn typo_flag_is_unknown_command_not_a_package() {
    let p = tmp_prefix("typoflag");
    let o = run(&p, &["-x"]);
    assert_eq!(o.code, 1);
    // Must NOT be misrouted into "install a package named -x".
    assert!(
        o.stderr.contains("unknown command '-x'"),
        "stderr: {}",
        o.stderr
    );
    assert!(!o.stderr.contains("install -x"), "stderr: {}", o.stderr);
}

#[test]
fn install_euglena_bad_flag_fails_clean_offline() {
    // Flag parsing happens before any network access, so this stays offline:
    // a bad flag must be a clean exit-1 error, never a panic (101).
    let p = tmp_prefix("euglena_badflag");
    let o = run(&p, &["install", "euglena", "--bogus"]);
    assert_eq!(o.code, 1, "should exit 1, not panic (101)");
    assert!(
        o.stderr.contains("unknown flag '--bogus'"),
        "stderr: {}",
        o.stderr
    );
}

#[test]
fn install_unknown_package_fails_clean() {
    let p = tmp_prefix("unknownpkg");
    let o = run(&p, &["install", "nonsense"]);
    assert_eq!(o.code, 1);
    assert!(
        o.stderr.contains("unknown package 'nonsense'"),
        "stderr: {}",
        o.stderr
    );
}

#[test]
fn dispatch_known_but_uninstalled_suggests_install() {
    let p = tmp_prefix("dispatch_known");
    let o = run(&p, &["code", "run", "x.code"]);
    assert_eq!(o.code, 1);
    assert!(
        o.stderr.contains("run `cdlvsm install code` first"),
        "stderr: {}",
        o.stderr
    );
}

#[test]
fn dispatch_unknown_name_points_at_help() {
    let p = tmp_prefix("dispatch_unknown");
    let o = run(&p, &["wat"]);
    assert_eq!(o.code, 1);
    assert!(o.stderr.contains("cdlvsm help"), "stderr: {}", o.stderr);
}

/// Dispatch sets CDLVSM_INVOKED_AS="cdlvsm <pkg>" so the dispatched tool's own
/// hints can say "cdlvsm <pkg> …" instead of a bare command name that doesn't
/// exist (cdlvsm never installs a bare `<pkg>` without --link).
#[test]
fn dispatch_sets_invoked_as_env_var() {
    let p = tmp_prefix("invoked_as");
    let pkg_dir = p.join("share/cdlvsm/packages/code/v1.0.0");
    fs::create_dir_all(&pkg_dir).unwrap();
    let fake = pkg_dir.join("code");
    fs::write(&fake, "#!/bin/sh\necho \"invoked_as=$CDLVSM_INVOKED_AS\"\n").unwrap();
    let mut perms = fs::metadata(&fake).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&fake, perms).unwrap();
    std::os::unix::fs::symlink("v1.0.0", p.join("share/cdlvsm/packages/code/current")).unwrap();

    let o = run(&p, &["code"]);
    assert_eq!(o.code, 0, "stderr: {}", o.stderr);
    assert!(
        o.stdout.contains("invoked_as=cdlvsm code"),
        "stdout: {}",
        o.stdout
    );
}

#[test]
fn list_empty_is_clean() {
    let p = tmp_prefix("list_empty");
    let o = run(&p, &["list"]);
    assert_eq!(o.code, 0);
    assert!(
        o.stdout.contains("No cdlvsm-managed packages installed."),
        "stdout: {}",
        o.stdout
    );
}

// --- offline: uninstall-safety regression (the most important property) -----
//
// Build a fake install tree by hand (no download), including BOTH a real
// cdlvsm-owned symlink and a FOREIGN plain file at the bare `code` name, then
// assert uninstall removes only what it owns.

fn make_fake_code_install(prefix: &Path) -> PathBuf {
    let ver_dir = prefix.join("share/cdlvsm/packages/code/v9.9.9");
    fs::create_dir_all(&ver_dir).unwrap();
    let real_bin = ver_dir.join("code");
    fs::write(&real_bin, b"#!/bin/sh\necho fake-code\n").unwrap();

    let current = prefix.join("share/cdlvsm/packages/code/current");
    std::os::unix::fs::symlink("v9.9.9", &current).unwrap();

    let bin = prefix.join("bin");
    fs::create_dir_all(&bin).unwrap();
    // cdlvsm-owned shim: a symlink into the package's own current/ dir.
    std::os::unix::fs::symlink(current.join("code"), bin.join("cdlvsm-code")).unwrap();
    bin
}

#[test]
fn uninstall_preserves_foreign_same_named_binary() {
    let p = tmp_prefix("safety");
    let bin = make_fake_code_install(&p);

    // A FOREIGN `code` — a plain file, not a symlink cdlvsm created (stands in
    // for VS Code's own `code` CLI sharing the bin dir).
    let foreign = bin.join("code");
    fs::write(&foreign, b"#!/bin/sh\necho vscode\n").unwrap();

    let o = run(&p, &["uninstall", "code"]);
    assert_eq!(o.code, 0, "stderr: {}", o.stderr);

    // The foreign file must be untouched...
    assert!(
        foreign.exists(),
        "foreign `code` was deleted — safety violation"
    );
    assert_eq!(fs::read(&foreign).unwrap(), b"#!/bin/sh\necho vscode\n");

    // ...the cdlvsm-owned shim must be gone...
    assert!(
        !bin.join("cdlvsm-code").exists(),
        "cdlvsm-code shim should have been removed"
    );
    // ...and the package dir gone.
    assert!(
        !p.join("share/cdlvsm/packages/code").exists(),
        "package dir should have been removed"
    );
}

#[test]
fn uninstall_removes_cdlvsm_owned_bare_link() {
    let p = tmp_prefix("safety_owned");
    let bin = make_fake_code_install(&p);

    // A bare `code` that IS a cdlvsm-owned symlink (the --link case) must be
    // removed, unlike a foreign file.
    let current = p.join("share/cdlvsm/packages/code/current");
    std::os::unix::fs::symlink(current.join("code"), bin.join("code")).unwrap();

    let o = run(&p, &["uninstall", "code"]);
    assert_eq!(o.code, 0, "stderr: {}", o.stderr);
    assert!(
        !bin.join("code").exists(),
        "cdlvsm-owned bare `code` link should have been removed"
    );
}

#[test]
fn uninstall_not_installed_fails_clean() {
    let p = tmp_prefix("uninstall_missing");
    let o = run(&p, &["uninstall", "code"]);
    assert_eq!(o.code, 1);
    assert!(
        o.stderr.contains("not installed via cdlvsm"),
        "stderr: {}",
        o.stderr
    );
}

#[test]
fn list_skips_broken_current_symlink() {
    let p = tmp_prefix("list_broken");
    // A package dir with NO current symlink — list must tolerate it, not crash.
    fs::create_dir_all(p.join("share/cdlvsm/packages/code/v1.0.0")).unwrap();
    let o = run(&p, &["list"]);
    assert_eq!(o.code, 0, "stderr: {}", o.stderr);
    assert!(
        o.stdout.contains("No cdlvsm-managed packages installed."),
        "stdout: {}",
        o.stdout
    );
}

// --- offline: upgrade -------------------------------------------------------
//
// The "already up to date" path is testable offline: a version pin
// (CDLVSM_CODE_VERSION) short-circuits the network `latest_tag` fetch, so
// upgrade resolves the tag without curl.

fn make_fake_upgrade_install(prefix: &Path, with_meta: bool) {
    let ver_dir = prefix.join("share/cdlvsm/packages/code/v9.9.9");
    fs::create_dir_all(&ver_dir).unwrap();
    let real_bin = ver_dir.join("code");
    fs::write(&real_bin, b"#!/bin/sh\necho fake-code\n").unwrap();
    std::os::unix::fs::symlink("v9.9.9", prefix.join("share/cdlvsm/packages/code/current"))
        .unwrap();

    if with_meta {
        fs::write(
            prefix.join("share/cdlvsm/packages/code/.cdlvsm"),
            "pkg=code\nrepo=codelovesme/code\nasset_base=code-sdk\nenv_var=CDLVSM_CODE_VERSION\nlabel=sdk\nlink=0\n",
        )
        .unwrap();
    }
}

#[test]
fn upgrade_unknown_package_fails_clean() {
    let p = tmp_prefix("upgrade_unknown");
    let o = run(&p, &["upgrade", "nonsense"]);
    assert_eq!(o.code, 1);
    assert!(
        o.stderr.contains("unknown package 'nonsense'"),
        "stderr: {}",
        o.stderr
    );
}

#[test]
fn upgrade_not_installed_fails_clean() {
    let p = tmp_prefix("upgrade_missing");
    let o = run(&p, &["upgrade", "code"]);
    assert_eq!(o.code, 1);
    assert!(o.stderr.contains("not installed"), "stderr: {}", o.stderr);
}

#[test]
fn upgrade_rejects_unknown_flag() {
    let p = tmp_prefix("upgrade_badflag");
    let o = run(&p, &["upgrade", "code", "--bogus"]);
    assert_eq!(o.code, 1, "should exit 1, not panic (101)");
    assert!(
        o.stderr.contains("unknown flag '--bogus'"),
        "stderr: {}",
        o.stderr
    );
}

#[test]
fn upgrade_rejects_multiple_packages() {
    let p = tmp_prefix("upgrade_multi");
    let o = run(&p, &["upgrade", "code", "euglena"]);
    assert_eq!(o.code, 1);
    assert!(
        o.stderr.contains("takes at most one package name"),
        "stderr: {}",
        o.stderr
    );
}

#[test]
fn upgrade_all_no_packages_is_clean() {
    let p = tmp_prefix("upgrade_all_empty");
    let o = run(&p, &["upgrade"]);
    assert_eq!(o.code, 0, "stderr: {}", o.stderr);
    assert!(
        o.stdout.contains("No cdlvsm-managed packages installed."),
        "stdout: {}",
        o.stdout
    );
}

#[test]
fn upgrade_already_up_to_date_offline() {
    let p = tmp_prefix("upgrade_uptodate");
    make_fake_upgrade_install(&p, true);
    // Pin to the installed version: upgrade resolves the tag without network
    // and must skip the download.
    let o = run_env(
        &p,
        &["upgrade", "code"],
        &[("CDLVSM_CODE_VERSION", "v9.9.9")],
    );
    assert_eq!(o.code, 0, "stderr: {}", o.stderr);
    assert!(
        o.stdout.contains("code is already up to date (v9.9.9)"),
        "stdout: {}",
        o.stdout
    );
}

#[test]
fn upgrade_missing_metadata_falls_back_to_defaults() {
    let p = tmp_prefix("upgrade_nometa");
    make_fake_upgrade_install(&p, false);
    let o = run_env(
        &p,
        &["upgrade", "code"],
        &[("CDLVSM_CODE_VERSION", "v9.9.9")],
    );
    assert_eq!(o.code, 0, "stderr: {}", o.stderr);
    assert!(
        o.stdout.contains("code is already up to date (v9.9.9)"),
        "stdout: {}",
        o.stdout
    );
    assert!(
        o.stderr.contains("no install metadata for 'code'"),
        "stderr: {}",
        o.stderr
    );
}

// --- network: real install of `code` from GitHub Releases ------------------

#[test]
fn real_install_dispatch_uninstall_roundtrip() {
    if std::env::var("CDLVSM_NETWORK_TESTS").as_deref() != Ok("1") {
        eprintln!("skipping network test (set CDLVSM_NETWORK_TESTS=1 to run)");
        return;
    }
    let p = tmp_prefix("real");

    let o = run(&p, &["install", "code"]);
    assert_eq!(o.code, 0, "install failed: {}", o.stderr);

    // list shows it
    let o = run(&p, &["list"]);
    assert!(o.stdout.contains("code"), "list: {}", o.stdout);

    // bare `code` must NOT exist without --link
    assert!(
        !p.join("bin/code").exists(),
        "bare code should not exist without --link"
    );
    assert!(
        p.join("bin/cdlvsm-code").exists(),
        "cdlvsm-code shim should exist"
    );

    // dispatch passthrough: cdlvsm code --version works
    let o = run(&p, &["code", "--version"]);
    assert_eq!(o.code, 0, "dispatch failed: {}", o.stderr);
    assert!(o.stdout.contains("Code"), "dispatch stdout: {}", o.stdout);

    // dispatch a real program
    let prog = p.join("prog.code");
    fs::write(&prog, b"a = 40 + 2\nassert a = 42\n").unwrap();
    let o = run(&p, &["code", "run", prog.to_str().unwrap()]);
    assert_eq!(o.code, 0, "program dispatch failed: {}", o.stderr);

    // uninstall
    let o = run(&p, &["uninstall", "code"]);
    assert_eq!(o.code, 0, "uninstall failed: {}", o.stderr);
    assert!(!p.join("bin/cdlvsm-code").exists());
}

#[test]
fn real_install_euglena_roundtrip() {
    if std::env::var("CDLVSM_NETWORK_TESTS").as_deref() != Ok("1") {
        eprintln!("skipping network test (set CDLVSM_NETWORK_TESTS=1 to run)");
        return;
    }
    let p = tmp_prefix("real_euglena");

    let o = run(&p, &["install", "euglena"]);
    assert_eq!(o.code, 0, "install failed: {}", o.stderr);

    let o = run(&p, &["list"]);
    assert!(o.stdout.contains("euglena"), "list: {}", o.stdout);
    assert!(
        p.join("bin/cdlvsm-euglena").exists(),
        "cdlvsm-euglena shim should exist"
    );
    assert!(
        !p.join("bin/euglena").exists(),
        "bare euglena should not exist without --link"
    );

    // dispatch passthrough: cdlvsm euglena --version works
    let o = run(&p, &["euglena", "--version"]);
    assert_eq!(o.code, 0, "dispatch failed: {}", o.stderr);
    assert!(
        o.stdout.contains("euglena"),
        "dispatch stdout: {}",
        o.stdout
    );

    let o = run(&p, &["uninstall", "euglena"]);
    assert_eq!(o.code, 0, "uninstall failed: {}", o.stderr);
    assert!(!p.join("bin/cdlvsm-euglena").exists());
}

#[test]
fn real_upgrade_roundtrip() {
    if std::env::var("CDLVSM_NETWORK_TESTS").as_deref() != Ok("1") {
        eprintln!("skipping network test (set CDLVSM_NETWORK_TESTS=1 to run)");
        return;
    }
    let p = tmp_prefix("real_upgrade");

    let o = run(&p, &["install", "code"]);
    assert_eq!(o.code, 0, "install failed: {}", o.stderr);

    // Install recorded metadata with the SDK asset base.
    let meta = p.join("share/cdlvsm/packages/code/.cdlvsm");
    let meta_body = fs::read_to_string(&meta).expect("install should write .cdlvsm metadata");
    assert!(
        meta_body.contains("asset_base=code-sdk"),
        "metadata: {}",
        meta_body
    );
    assert!(
        meta_body.contains("env_var=CDLVSM_CODE_VERSION"),
        "metadata: {}",
        meta_body
    );

    // Upgrade right after install: already at latest, so no re-download.
    let o = run(&p, &["upgrade", "code"]);
    assert_eq!(o.code, 0, "upgrade failed: {}", o.stderr);
    assert!(
        o.stdout.contains("code is already up to date"),
        "stdout: {}",
        o.stdout
    );

    // Bare `upgrade` covers all installed packages.
    let o = run(&p, &["upgrade"]);
    assert_eq!(o.code, 0, "upgrade-all failed: {}", o.stderr);
    assert!(
        o.stdout.contains("code is already up to date"),
        "stdout: {}",
        o.stdout
    );

    // Dispatch still works after the upgrade no-op.
    let o = run(&p, &["code", "--version"]);
    assert_eq!(o.code, 0, "dispatch failed: {}", o.stderr);

    let o = run(&p, &["uninstall", "code"]);
    assert_eq!(o.code, 0, "uninstall failed: {}", o.stderr);
    assert!(!p.join("share/cdlvsm/packages/code").exists());
}
