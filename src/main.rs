//! cdlvsm — package manager and dispatcher for codelovesme's first-party CLI
//! tools.
//!
//! Two jobs:
//!   1. Package management: `install`/`uninstall`/`list`.
//!   2. Dispatch: `cdlvsm <pkg> <args...>` transparently execs the installed
//!      binary, forwarding argv and inheriting stdio/exit code/signals.
//!
//! Dispatch is done with a manual argv match (not clap) because the first arg
//! is either one of a fixed set of built-ins OR an arbitrary installed package
//! name with its own arbitrary trailing args — a shape a static subcommand
//! enum doesn't fit.
//!
//! RESERVED NAMES: `install`, `uninstall`, `upgrade`, `list`, `help`, `-h`,
//! `--help`, `-v`, `--version` are built-ins and can never be package names.
//! A package named one of these would be permanently unreachable via dispatch.

mod download;
mod error;
mod package;
mod paths;

use std::os::unix::process::CommandExt;
use std::process::Command;

use error::Result;
use package::{InstallOpts, Package, Tier};

const VERSION: &str = concat!("cdlvsm ", env!("CARGO_PKG_VERSION"));

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match run(&args) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("error: {}", e.0);
            std::process::exit(1);
        }
    }
}

fn run(args: &[String]) -> Result<i32> {
    let cmd = match args.get(1) {
        Some(c) => c.as_str(),
        None => {
            print_usage();
            return Ok(1);
        }
    };

    match cmd {
        "-h" | "--help" | "help" => {
            print_usage();
            Ok(0)
        }
        "-v" | "--version" => {
            println!("{VERSION}");
            Ok(0)
        }
        "install" => cmd_install(&args[2..]),
        "uninstall" => cmd_uninstall(&args[2..]),
        "upgrade" => cmd_upgrade(&args[2..]),
        "list" => package::list().map(|_| 0),
        // A flag-looking first arg is never a package name — a typo like
        // `cdlvsm -x` should be "unknown command", not "install a package
        // named -x".
        other if other.starts_with('-') => {
            eprintln!("error: unknown command '{other}'");
            print_usage();
            Ok(1)
        }
        // Anything else: treat as an installed package to dispatch to.
        pkg => dispatch(pkg, &args[2..]),
    }
}

fn cmd_install(rest: &[String]) -> Result<i32> {
    let name = match rest.first() {
        Some(n) => n.as_str(),
        None => {
            eprintln!("error: missing package name");
            print_usage();
            return Ok(1);
        }
    };

    let pkg = match Package::parse(name) {
        Some(p) => p,
        None => {
            eprintln!("error: unknown package '{name}'");
            print_usage();
            return Ok(1);
        }
    };

    let mut tier = Tier::Sdk;
    let mut link = false;
    for arg in &rest[1..] {
        match arg.as_str() {
            "--runtime" => tier = Tier::Runtime,
            "--sdk" => tier = Tier::Sdk,
            "--link" => link = true,
            other => {
                eprintln!("error: unknown flag '{other}' for 'cdlvsm install {name}'");
                return Ok(1);
            }
        }
    }

    pkg.install(&InstallOpts { tier, link })?;
    Ok(0)
}

fn cmd_uninstall(rest: &[String]) -> Result<i32> {
    let name = match rest.first() {
        Some(n) => n.as_str(),
        None => {
            eprintln!("error: missing package name");
            print_usage();
            return Ok(1);
        }
    };
    package::uninstall(name)?;
    Ok(0)
}

/// `cdlvsm upgrade [package]` — update one package, or all installed
/// packages when no name is given. No flags: install settings (tier, --link)
/// come from the recorded metadata; to change them, re-run `cdlvsm install`.
fn cmd_upgrade(rest: &[String]) -> Result<i32> {
    // Any flag-looking arg is an error — upgrade takes no flags.
    for arg in rest {
        if arg.starts_with('-') {
            eprintln!("error: unknown flag '{arg}' for 'cdlvsm upgrade'");
            print_usage();
            return Ok(1);
        }
    }
    if rest.len() > 1 {
        eprintln!("error: 'cdlvsm upgrade' takes at most one package name");
        print_usage();
        return Ok(1);
    }
    match rest.first() {
        Some(name) => package::upgrade(name).map(|_| 0),
        None => package::upgrade_all().map(|_| 0),
    }
}

/// Dispatch to an installed package's binary, replacing this process.
fn dispatch(name: &str, rest: &[String]) -> Result<i32> {
    let target = paths::dispatch_target(name);
    if !target.exists() {
        // Distinguish "known package, not installed" from "unknown name".
        if Package::parse(name).is_some() {
            eprintln!("error: '{name}' is not installed — run `cdlvsm install {name}` first.");
        } else {
            eprintln!(
                "error: '{name}' is not an installed package and not a cdlvsm command.\n\
                 Run `cdlvsm help` to see built-in commands."
            );
        }
        return Ok(1);
    }

    // exec() replaces this process image: same PID, so exit code and signal
    // handling (Ctrl-C, etc.) are exactly what running the target directly
    // would give — no translation logic. Default Command inherits stdin/
    // stdout/stderr, which is the transparent passthrough we want; do NOT
    // add any .stdout()/.stderr() redirection here.
    //
    // CDLVSM_INVOKED_AS tells the dispatched tool how it was launched, so its
    // own "next steps" hints can point back at `cdlvsm <pkg> …` rather than a
    // bare command name that may not exist (cdlvsm installs a `cdlvsm-<pkg>`
    // shim, not a bare `<pkg>`).
    //
    // Nothing is printed before this point, so there's no buffered output to
    // flush — exec() does not run Rust destructors/atexit.
    let err = Command::new(&target)
        .env("CDLVSM_INVOKED_AS", format!("cdlvsm {name}"))
        .args(rest)
        .exec();
    // exec() only returns on failure.
    Err(error::CdlvsmError(format!(
        "failed to exec {}: {err}",
        target.display()
    )))
}

fn print_usage() {
    println!("{VERSION}");
    println!();
    println!("Usage:");
    println!("  cdlvsm install <package> [--runtime] [--link]");
    println!("  cdlvsm uninstall <package>");
    println!("  cdlvsm upgrade [package]");
    println!("  cdlvsm list");
    println!("  cdlvsm <package> <args...>       run an installed package's binary");
    println!();
    println!("Packages:");
    println!("  code      the Code language toolchain (SDK tier by default;");
    println!("            --runtime for the smaller LLVM-free interpreter-only build)");
    println!("  euglena   the Euglena app CLI (scaffold/run/build Euglena apps)");
    println!();
    println!("Flags:");
    println!("  --runtime   install code's Runtime tier instead of the default SDK tier");
    println!("  --link      also create a bare `<package>` command in $PREFIX/bin (opt-in;");
    println!("              for `code` this avoids colliding with VS Code's own `code` CLI)");
    println!();
    println!("  `upgrade` takes no flags — it reuses the recorded install settings.");
    println!("  To change tier/--link, re-run `cdlvsm install <package>` with flags.");
    println!();
    println!("Env:");
    println!("  PREFIX                   install root (default: $HOME/.local)");
    println!("  CDLVSM_CODE_VERSION      pin code's version instead of latest (e.g. v0.3.0)");
    println!("  CDLVSM_EUGLENA_VERSION   pin euglena's version instead of latest");
}
