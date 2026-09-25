//! Apps in the desktop's own launcher — GNOME's search, KDE's menu,
//! Spotlight — for the packages that are apps rather than command-line
//! tools.
//!
//! Each desktop has one standard, per-user place an app says it is there:
//!
//! - Linux (every XDG desktop: GNOME, KDE, XFCE, Cinnamon, …): a desktop
//!   entry, `$XDG_DATA_HOME/applications/codelovesme-<pkg>.desktop`.
//! - macOS: an app bundle, `~/Applications/<Name>.app`, which Spotlight and
//!   Launchpad find by themselves.
//!
//! (Windows would be a Start Menu shortcut; cdlvsm does not run there yet.)
//!
//! What an app is called and how it starts comes from an `app.info` in its
//! release, `key=value` lines —
//!
//! ```text
//! name=codelovesme IDE
//! comment=A terminal IDE for the code language
//! terminal=true          # it draws in a terminal: open one to run it in
//! icon=icon.png          # a file in the release, or a theme icon's name
//! categories=Development;IDE;
//! keywords=editor;code;
//! ```
//!
//! — or, for the packages cdlvsm knows, from its own defaults below. A
//! command-line tool (`code`, `euglena`) has neither and gets no entry.
//! The entry starts the app through its `cdlvsm-<pkg>` shim, so an upgrade
//! never leaves it pointing at a version that is gone.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{fail, Result};
use crate::paths;

#[derive(Clone, Debug, PartialEq)]
pub struct App {
    pub name: String,
    pub comment: String,
    /// It runs inside a terminal, so its entry opens one to run it in.
    pub terminal: bool,
    /// A theme icon's name, or a path (made absolute when it is the
    /// release's own file).
    pub icon: String,
    pub categories: String,
    pub keywords: String,
}

/// The apps cdlvsm knows, for releases without an `app.info`.
fn builtin(pkg: &str) -> Option<App> {
    let app = |name: &str, comment: &str, terminal, icon: &str, categories: &str, keywords: &str| App {
        name: name.into(),
        comment: comment.into(),
        terminal,
        icon: icon.into(),
        categories: categories.into(),
        keywords: keywords.into(),
    };
    match pkg {
        "ide" => Some(app(
            "codelovesme IDE",
            "A terminal IDE for the code language",
            true,
            "accessories-text-editor",
            "Development;IDE;TextEditor;Utility;",
            "editor;code;euglena;codelovesme;",
        )),
        "console" => Some(app(
            "codelovesme console",
            "A terminal in a window of its own",
            false,
            "utilities-terminal",
            "System;TerminalEmulator;",
            "terminal;shell;command;codelovesme;",
        )),
        _ => None,
    }
}

/// `app.info`'s text read over `base` (the defaults, if any). Unknown keys
/// and `#` comments are ignored; a missing `name` means it is not an app.
pub fn parse_info(text: &str, base: Option<App>) -> Option<App> {
    let mut app = base.unwrap_or(App {
        name: String::new(),
        comment: String::new(),
        terminal: false,
        icon: String::new(),
        categories: "Utility;".into(),
        keywords: String::new(),
    });
    for line in text.lines() {
        let line = line.split(" #").next().unwrap_or("").trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim().to_string();
        match key.trim() {
            "name" => app.name = value,
            "comment" => app.comment = value,
            "terminal" => app.terminal = value == "true",
            "icon" => app.icon = value,
            "categories" => app.categories = value,
            "keywords" => app.keywords = value,
            _ => {}
        }
    }
    (!app.name.is_empty()).then_some(app)
}

/// The app in the release installed at `dir` for `pkg`, if it is one.
pub fn app_for(pkg: &str, dir: &Path) -> Option<App> {
    let mut app = match fs::read_to_string(dir.join("app.info")) {
        Ok(text) => parse_info(&text, builtin(pkg))?,
        Err(_) => builtin(pkg)?,
    };
    // A file of the release, reached through `current` so it follows upgrades.
    if !app.icon.is_empty() && !app.icon.contains('/') && dir.join(&app.icon).is_file() {
        app.icon = paths::current_link(pkg).join(&app.icon).display().to_string();
    }
    Some(app)
}

/// A value for a desktop entry: its escapes, on one line.
fn entry_value(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\n', " ")
}

/// A command-line argument for a desktop entry's `Exec`, quoted as the
/// spec asks when it needs to be.
fn exec_arg(s: &str) -> String {
    let plain = s.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+".contains(c));
    if plain {
        return s.to_string();
    }
    let mut out = String::from("\"");
    for c in s.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out.replace('%', "%%")
}

/// The XDG desktop entry for `app`, started by `exec`.
pub fn desktop_entry(pkg: &str, app: &App, exec: &Path) -> String {
    let mut out = String::from("[Desktop Entry]\nType=Application\n");
    out += &format!("Name={}\n", entry_value(&app.name));
    if !app.comment.is_empty() {
        out += &format!("Comment={}\n", entry_value(&app.comment));
    }
    out += &format!("Exec={}\n", exec_arg(&exec.display().to_string()));
    if !app.icon.is_empty() {
        out += &format!("Icon={}\n", entry_value(&app.icon));
    }
    out += &format!("Terminal={}\n", app.terminal);
    out += &format!("Categories={}\n", entry_value(&app.categories));
    if !app.keywords.is_empty() {
        out += &format!("Keywords={}\n", entry_value(&app.keywords));
    }
    out += &format!("StartupNotify={}\n", !app.terminal);
    // Whose it is, so uninstall removes only what cdlvsm wrote.
    out += &format!("X-cdlvsm-Package={pkg}\n");
    out
}

/// A string for a shell script, in single quotes.
fn sh_quoted(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The macOS bundle's program: the shim itself, or, for a terminal app,
/// Terminal.app told to run it.
pub fn mac_launcher(app: &App, exec: &Path) -> String {
    let exec = exec.display().to_string();
    if app.terminal {
        let command = format!("exec {}", sh_quoted(&exec)).replace('\\', "\\\\").replace('"', "\\\"");
        format!(
            "#!/bin/sh\nosascript -e 'tell application \"Terminal\" to do script \"{}\"' -e 'tell application \"Terminal\" to activate'\n",
            command.replace('\'', "'\\''")
        )
    } else {
        format!("#!/bin/sh\nexec {} \"$@\"\n", sh_quoted(&exec))
    }
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// The macOS bundle's Info.plist.
pub fn mac_plist(pkg: &str, app: &App) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n  <key>CFBundleName</key><string>{name}</string>\n  <key>CFBundleDisplayName</key><string>{name}</string>\n  <key>CFBundleIdentifier</key><string>me.codeloves.{pkg}</string>\n  <key>CFBundleExecutable</key><string>{pkg}</string>\n  <key>CFBundlePackageType</key><string>APPL</string>\n  <key>CFBundleShortVersionString</key><string>1</string>\n  <key>LSMinimumSystemVersion</key><string>10.13</string>\n  <key>X-cdlvsm-Package</key><string>{pkg}</string>\n</dict>\n</plist>\n",
        name = xml(&app.name)
    )
}

fn applications_dir() -> PathBuf {
    let data = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
            home.join(".local/share")
        });
    data.join("applications")
}

fn mac_app_dir(app: &App) -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    home.join("Applications").join(format!("{}.app", app.name.replace('/', "-")))
}

fn write(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| crate::error::CdlvsmError(format!("mkdir {}: {e}", parent.display())))?;
    }
    fs::write(path, text).map_err(|e| crate::error::CdlvsmError(format!("write {}: {e}", path.display())))
}

/// `pkg` put in the desktop's launcher, if it is an app. Answers where.
pub fn register(pkg: &str, dir: &Path) -> Result<Option<PathBuf>> {
    let Some(app) = app_for(pkg, dir) else { return Ok(None) };
    let exec = paths::bin_dir().join(format!("cdlvsm-{pkg}"));
    if cfg!(target_os = "macos") {
        let bundle = mac_app_dir(&app);
        let program = bundle.join("Contents/MacOS").join(pkg);
        write(&bundle.join("Contents/Info.plist"), &mac_plist(pkg, &app))?;
        write(&program, &mac_launcher(&app, &exec))?;
        crate::download::make_executable(&program)?;
        return Ok(Some(bundle));
    }
    if cfg!(unix) {
        let entry = applications_dir().join(format!("codelovesme-{pkg}.desktop"));
        write(&entry, &desktop_entry(pkg, &app, &exec))?;
        // Menus that cache (update-desktop-database); nothing if it is not there.
        let _ = std::process::Command::new("update-desktop-database")
            .arg(applications_dir())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        return Ok(Some(entry));
    }
    fail("apps in the desktop's launcher are made on Linux and macOS only")
}

/// `pkg`'s launcher entry taken away — only one cdlvsm wrote.
pub fn unregister(pkg: &str) {
    let marker = format!("X-cdlvsm-Package={pkg}");
    let entry = applications_dir().join(format!("codelovesme-{pkg}.desktop"));
    if fs::read_to_string(&entry).map(|t| t.contains(&marker)).unwrap_or(false) {
        let _ = fs::remove_file(&entry);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let mac_marker = format!("<key>X-cdlvsm-Package</key><string>{pkg}</string>");
    if let Ok(apps) = fs::read_dir(home.join("Applications")) {
        for bundle in apps.flatten() {
            let plist = bundle.path().join("Contents/Info.plist");
            if fs::read_to_string(&plist).map(|t| t.contains(&mac_marker)).unwrap_or(false) {
                let _ = fs::remove_dir_all(bundle.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_apps_and_tools() {
        assert!(builtin("ide").unwrap().terminal);
        assert!(!builtin("console").unwrap().terminal);
        assert!(builtin("code").is_none() && builtin("euglena").is_none());
    }

    #[test]
    fn app_info_overrides_the_defaults() {
        let app = parse_info("# mine\nname=Mine\nterminal=true  # in a terminal\nicon=icon.png\nwhat=ever\n", builtin("console")).unwrap();
        assert_eq!(app.name, "Mine");
        assert!(app.terminal);
        assert_eq!(app.icon, "icon.png");
        assert_eq!(app.categories, "System;TerminalEmulator;");
        assert!(parse_info("comment=no name\n", None).is_none());
    }

    #[test]
    fn a_desktop_entry() {
        let entry = desktop_entry("ide", &builtin("ide").unwrap(), Path::new("/home/a b/.local/bin/cdlvsm-ide"));
        assert!(entry.starts_with("[Desktop Entry]\nType=Application\nName=codelovesme IDE\n"));
        assert!(entry.contains("\nExec=\"/home/a b/.local/bin/cdlvsm-ide\"\n"));
        assert!(entry.contains("\nTerminal=true\n"));
        assert!(entry.contains("\nIcon=accessories-text-editor\n"));
        assert!(entry.contains("\nX-cdlvsm-Package=ide\n"));
        let plain = desktop_entry("console", &builtin("console").unwrap(), Path::new("/p/cdlvsm-console"));
        assert!(plain.contains("\nExec=/p/cdlvsm-console\n") && plain.contains("\nTerminal=false\n"));
    }

    #[test]
    fn a_mac_bundle() {
        let term = mac_launcher(&builtin("ide").unwrap(), Path::new("/Users/me/.local/bin/cdlvsm-ide"));
        assert!(term.contains("tell application \"Terminal\" to do script"), "{term}");
        assert!(term.contains("/Users/me/.local/bin/cdlvsm-ide"));
        let window = mac_launcher(&builtin("console").unwrap(), Path::new("/Users/me/.local/bin/cdlvsm-console"));
        assert_eq!(window, "#!/bin/sh\nexec '/Users/me/.local/bin/cdlvsm-console' \"$@\"\n");
        let plist = mac_plist("console", &builtin("console").unwrap());
        assert!(plist.contains("<key>CFBundleExecutable</key><string>console</string>"));
        assert!(plist.contains("<string>codelovesme console</string>"));
    }
}
