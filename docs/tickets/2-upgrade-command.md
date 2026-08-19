# 2 — `cdlvsm upgrade` — package upgrade support

Status: Implemented.

## What this is

`cdlvsm upgrade` updates installed packages to their latest (or
version-pin env var) release:

```
cdlvsm upgrade code        # update one package
cdlvsm upgrade             # update every installed package
```

Ticket 1 deliberately shipped *without* an update command — "re-running
`cdlvsm install <pkg>` … is the update." That works, but it has a real
footgun: `install` re-applies **default** settings, so re-installing a
`code --runtime --link` package silently resets it to the SDK tier with no
bare link. `upgrade` fixes that by recording how a package was installed and
reusing the exact settings.

## Design

### Install metadata (`.cdlvsm`)

`install_release` now writes a small key=value file at the package-dir level
on every successful install/upgrade:

```
$PREFIX/share/cdlvsm/packages/<pkg>/.cdlvsm
    pkg=code
    repo=codelovesme/code
    asset_base=code-sdk
    env_var=CDLVSM_CODE_VERSION
    label=sdk
    link=0
```

- Plain key=value lines — zero dependencies, matching the curl/tar
  philosophy. Unknown keys are ignored on read (forward compatibility).
- Package-dir level, so `uninstall`'s `rm_rf` removes it automatically and
  `list` (which only iterates subdirs) never sees it.
- `label` is empty for `euglena`; `link` is `0`/`1`.

### `upgrade(name)` (`src/package.rs`)

1. Package dir missing → known name: "not installed — run `cdlvsm install`
   first"; unknown name: "unknown package" (exit 1).
2. Current version from the `current` symlink. Broken/missing symlink is
   tolerated — the upgrade proceeds with no "old" version and self-heals.
3. Settings from `.cdlvsm`. Missing + name known to the `Package` enum →
   enum defaults (SDK, no link) with a stderr warning (one-time; the
   upgrade rewrites the metadata). Missing + unknown name → clean failure.
4. `resolve_tag(meta.env_var, meta.repo)` — the same env-pin-else-latest
   path `install` uses.
5. Resolved tag == current version → `"{name} is already up to date
   ({tag})"`, no download.
6. Otherwise the shared `install_release()` path, with `verb: "Upgraded"`
   and the old version for the final message:
   `Upgraded code (sdk) v0.2.0 -> v0.3.0`.

### `upgrade_all()`

Iterates the sorted package dirs (same pattern as `list`), runs `upgrade`
per package, reports per-package errors and continues, and fails (exit 1)
if any package failed. Empty/missing root → "No cdlvsm-managed packages
installed." (exit 0).

### CLI (`src/main.rs`)

`upgrade` is a reserved built-in name. `cmd_upgrade` takes at most one
package-name arg and **no flags** — any `--x` is a clean exit-1 "unknown
flag" error; settings come from the recorded metadata, and changing
tier/`--link` means re-running `cdlvsm install` (documented in usage and
README).

## Scope / decisions

- **No flags on `upgrade`** — by design; see above.
- **Tag comparison is string equality** — no semver parsing (zero-dep).
  Pinning an *older* version via the env var proceeds with the install
  (explicit user intent); only an exact match skips.
- **No cdlvsm self-upgrade** — `install.sh` + `CDLVSM_CLI_VERSION` cover
  that.
- **Old installs without metadata** — known names fall back to enum
  defaults with a warning; the upgrade rewrites the metadata.
- Linux x86_64 only (unchanged); the uninstall-safety property is untouched.

## Tests

Offline (always run): unknown package, not-installed, unknown flag,
multiple package names, no-packages, already-up-to-date (fake tree +
hand-written `.cdlvsm` + `CDLVSM_CODE_VERSION` pin, so no network), and
missing-metadata fallback. Network (gated `CDLVSM_NETWORK_TESTS=1`):
`real_upgrade_roundtrip` — install → assert `.cdlvsm` contents → upgrade
("already up to date") → bare upgrade → dispatch → uninstall.
