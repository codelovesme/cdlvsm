# cdlvsm

Package manager and dispatcher CLI for [codelovesme](https://github.com/codelovesme)'s
first-party command-line tools.

`cdlvsm` installs a tool, keeps it under a versioned directory, and then
transparently runs it for you:

```bash
cdlvsm install code
cdlvsm code run hello.code      # runs the installed `code` binary
```

`cdlvsm <package> <args...>` execs the installed binary directly, forwarding
argv and inheriting stdin/stdout/stderr, exit code, and signals — so
`cdlvsm code ...` behaves exactly like running `code ...` would.

## Install

Linux x86_64 only, for now.

```bash
curl -sSf https://raw.githubusercontent.com/codelovesme/cdlvsm-cli/main/install.sh | sh
```

Installs the `cdlvsm` binary to `~/.local/bin` (override with `PREFIX=...`;
pin a version with `CDLVSM_CLI_VERSION=v0.x.y`). Add `~/.local/bin` to your
`PATH` if it isn't already.

## Usage

```
cdlvsm install <package> [--runtime] [--link]
cdlvsm uninstall <package>
cdlvsm upgrade [package]
cdlvsm update
cdlvsm list
cdlvsm <package> <args...>       run an installed package's binary
```

### Packages

| Package   | What it is |
|-----------|------------|
| `code`    | the [Code](https://github.com/codelovesme/code) language toolchain |
| `euglena` | the [euglena](https://github.com/codelovesme/euglena-cli) app CLI (scaffold/run/build Euglena apps) |

`euglena` runs apps through the `code` interpreter, so `cdlvsm install
euglena` also installs `code` (SDK tier, no `--link`) if it isn't already
installed, and points euglena at cdlvsm's `code` shim — no separate `install
code` or `euglena code set` step needed. That wiring keeps working across
`cdlvsm upgrade code` too, since the shim always tracks whichever version is
current. To use a different `code` build instead, run `euglena code set
<path>` yourself afterwards.

### `code` install flags

- `--runtime` — install the smaller, LLVM-free interpreter-only tier instead
  of the default SDK (which also has the native/wasm compiler). Tiers only
  exist for releases that ship separate per-tier builds (`code` did through
  v0.4.1); from v0.5.0 on there is a single build, and cdlvsm installs it with
  a note regardless of the tier flag.
- `--link` — additionally create a bare `code` command in `$PREFIX/bin`. By
  default `cdlvsm install code` only creates a `cdlvsm-code` shim, **not** a
  bare `code`, because `code` collides with VS Code's own `code` CLI on
  Linux. Opt into the bare name only if you know you want it.

### Environment

- `PREFIX` — install root (default `$HOME/.local`). Packages live under
  `$PREFIX/share/cdlvsm/packages/<pkg>/<version>/`; shims under `$PREFIX/bin`.
- `CDLVSM_CODE_VERSION` / `CDLVSM_EUGLENA_VERSION` — pin a package's version
  instead of fetching the latest release (e.g. `v0.3.0`).
- `CDLVSM_CLI_VERSION` — pin which `cdlvsm` build to fetch, for both
  `install.sh` and `cdlvsm update`.

### Updating

`cdlvsm upgrade` updates packages to their latest (or version-pin) release:

```
cdlvsm upgrade code        # update one package
cdlvsm upgrade             # update every installed package
```

`upgrade` reuses the settings the package was installed with (recorded in
`$PREFIX/share/cdlvsm/packages/<pkg>/.cdlvsm`) — a `code --runtime --link`
install stays a `--runtime` install with its bare `code` link. If a package
is already at the latest (or pinned) version, it's reported as "already up
to date" and skipped. `upgrade` takes no flags; to change tier or `--link`,
re-run `cdlvsm install <package>` with the new flags.

`cdlvsm update` is the same idea for `cdlvsm` itself — distinct from
`upgrade`, which only touches installed packages. It downloads the latest
(or `CDLVSM_CLI_VERSION`-pinned) release and replaces the running binary in
place; no shell restart needed, the new version takes effect immediately.

### Uninstall safety

`cdlvsm uninstall <pkg>` removes a shim in `$PREFIX/bin` **only if** it's a
symlink cdlvsm itself created (one that resolves back into the package's own
directory). A same-named binary you installed some other way — e.g. VS Code's
real `code` — is never touched.

## Building from source

```bash
cargo build --release       # ./target/release/cdlvsm
cargo test                  # offline tests
CDLVSM_NETWORK_TESTS=1 cargo test   # also runs the real install-from-GitHub test
```

No LLVM or other system dependencies — `cdlvsm` shells out to `curl`/`tar` at
runtime (both required on `PATH`) rather than embedding an HTTP/archive stack.

## License

GPL-3.0-or-later — see [LICENSE](./LICENSE).
