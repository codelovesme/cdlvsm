# 1 — `cdlvsm` package-manager & dispatcher CLI

Status: Implemented and shipped (initial v0.1.0).

## What this is

`cdlvsm` is the package manager for codelovesme's first-party CLI tools. It
does two jobs:

1. **Package management** — `install` / `uninstall` / `list`.
2. **Dispatch** — `cdlvsm <package> <args...>` transparently runs an
   installed tool's binary.

It began as a POSIX-`sh` proof-of-concept living inside the
[`codelovesme/code`](https://github.com/codelovesme/code) repo (that repo's
tickets T32/T33). It was extracted here and rewritten in Rust once it needed
to become a real dispatcher serving more than one tool — the same
extraction path `code` itself took out of the `euglena-platform` monorepo.

> **Naming note.** "cdlvsm" is also, separately, a pre-existing informal
> *local checkout nickname* for the `codelovesme/euglena-platform` monorepo
> (visible in that repo's own `dev.sh` and tickets, and referenced as
> `../cdlvsm/code-vscode` in `code`'s ticket 16). That usage is unrelated to
> this CLI — this repo, `codelovesme/cdlvsm-cli`, is the actual `cdlvsm`
> command-line tool.

## Design

### Dispatch (`src/main.rs`)

Manual `std::env::args()` matching, not `clap` — because the first argument
is either one of a fixed set of built-ins OR an arbitrary installed package
name carrying its own arbitrary trailing args, a shape a static subcommand
enum doesn't fit.

- **Reserved built-in names** (can never be package names): `install`,
  `uninstall`, `list`, `help`, `-h`, `--help`, `-v`, `--version`. A package
  named one of these would be permanently unreachable via dispatch —
  documented as a hard constraint.
- Any first arg starting with `-` is treated as a (bad) command, never a
  package name — so a typo like `cdlvsm -x` gives "unknown command", not a
  nonsensical "install a package named -x".
- Anything else is a package name: resolve
  `$PREFIX/share/cdlvsm/packages/<name>/current/<name>` and, if it exists,
  hand off to it.

**Passthrough uses `exec()`** (`std::os::unix::process::CommandExt::exec`),
replacing the process image rather than spawn+wait. This makes exit codes
and signal handling (Ctrl-C, etc.) exactly correct for free — a spawn+wait
approach would need manual `ExitStatusExt::signal()` translation to avoid
silently reporting exit code 1 for a signal-killed child. Stdio is inherited
(no redirection) — that's the transparent passthrough. `exec()` only returns
on failure, which is handled as a clean error, never a panic.

### Package registry (`src/package.rs`)

`enum Package { Code, Euglena }`, consulted only by install/uninstall/list —
dispatch is purely filesystem-based and never touches it, so an installed
package dispatches even if a future cdlvsm wouldn't recognize its name. Both
packages share one `install_release()` path driven by a small `ReleaseSpec`
(repo, asset-name base, version env var, optional tier label); the two
`install_*` functions just fill it in.

- `code` downloads `code-{sdk,runtime}-<tag>-x86_64-linux.tar.gz` from
  `codelovesme/code`'s Releases (`CDLVSM_CODE_VERSION` pins a version, else
  latest), extracts under a versioned dir, repoints a `current` symlink, and
  creates a `cdlvsm-code` shim (plus a bare `code` only with `--link`).
- `euglena` downloads `euglena-<tag>-x86_64-linux.tar.gz` from
  [`codelovesme/euglena-cli`](https://github.com/codelovesme/euglena-cli)'s
  Releases (`CDLVSM_EUGLENA_VERSION` pins a version) through the same
  `install_release()` path — added once that repo was extracted from the
  private `euglena-platform` monorepo and given a real release (see that
  repo's `docs/tickets/done/1-extract-euglena-cli-to-own-repo.md`). Verified
  end-to-end via the `real_install_euglena_roundtrip` network test.

### Download seam (`src/download.rs`)

Shells out to `curl`/`tar` via `std::process::Command` rather than embedding
`reqwest`/`flate2`/`tar` crates — zero dependencies, mirrors the proven
shell prototype 1:1, and matches the org's existing dependency-minimalism.
Kept behind `download()`/`extract()`/`latest_tag()` functions so the
implementation can be swapped or faked later without touching install logic.
The GitHub API tag extraction is deliberately the same lightweight
string-scan the shell version used, not a JSON dependency.

### Uninstall safety — the property that matters most

`uninstall` removes `$PREFIX/bin/cdlvsm-<pkg>` and `$PREFIX/bin/<pkg>` **only
if** each is a symlink whose target resolves back inside that package's own
directory. A same-named binary cdlvsm didn't create — e.g. VS Code's real
`code` sharing the bin dir — is never touched. This was only manually
verified for the shell prototype; here it's a real regression test
(`tests/cli.rs::uninstall_preserves_foreign_same_named_binary`) building a
fake install tree with both a cdlvsm-owned symlink and a foreign plain file,
asserting only the owned one is removed. Runs offline on every CI push.

## No `update` command

Re-running `cdlvsm install <pkg>` re-fetches latest/pinned and repoints
`current` — that is update. A separate subcommand would be redundant.

## Distribution

- `install.sh` bootstraps `cdlvsm` itself from this repo's GitHub Releases
  (`CDLVSM_CLI_VERSION` pins the cdlvsm build — deliberately distinct from
  `CDLVSM_CODE_VERSION`, which cdlvsm reads at runtime to pin `code`'s
  version).
- `.github/workflows/ci.yml`: fmt + clippy + build + offline tests on every
  push/PR. No LLVM or other system deps.
- `.github/workflows/release.yml`: tag-push `v*` (plus a `workflow_dispatch`
  dry-run) builds `--release`, sanity-checks the binary, stages
  `cdlvsm`+`LICENSE`+`README.md`, and uploads `cdlvsm-<tag>-x86_64-linux.tar.gz`
  as a GitHub Release asset.

## Scope / not done

- Linux x86_64 only (matches the tools it installs).
- `code` and `euglena` are both really installable now.
- No package index / third-party packages — a two-entry enum is the whole
  registry, which is right for a two-tool ecosystem today.
