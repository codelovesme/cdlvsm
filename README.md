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
cdlvsm list
cdlvsm <package> <args...>       run an installed package's binary
```

### Packages

| Package   | Status | What it is |
|-----------|--------|------------|
| `code`    | available | the [Code](https://github.com/codelovesme/code) language toolchain |
| `euglena` | not published yet | `cdlvsm install euglena` errors clearly until it ships |

### `code` install flags

- `--runtime` — install the smaller, LLVM-free interpreter-only tier instead
  of the default SDK (which also has the native/wasm compiler).
- `--link` — additionally create a bare `code` command in `$PREFIX/bin`. By
  default `cdlvsm install code` only creates a `cdlvsm-code` shim, **not** a
  bare `code`, because `code` collides with VS Code's own `code` CLI on
  Linux. Opt into the bare name only if you know you want it.

### Environment

- `PREFIX` — install root (default `$HOME/.local`). Packages live under
  `$PREFIX/share/cdlvsm/packages/<pkg>/<version>/`; shims under `$PREFIX/bin`.
- `CDLVSM_CODE_VERSION` — pin `code`'s version instead of fetching the latest
  release (e.g. `v0.3.0`). Distinct from `CDLVSM_CLI_VERSION`, which pins
  which `cdlvsm` build the installer script fetches.

### Updating

There's no separate `update` command — re-running `cdlvsm install <pkg>`
re-fetches the latest (or `CDLVSM_CODE_VERSION`-pinned) release and repoints
the package's `current` symlink. That is the update.

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
