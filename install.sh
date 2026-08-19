#!/usr/bin/env sh
# Bootstrap installer for `cdlvsm` itself, from GitHub Releases.
#
#   curl -sSf https://raw.githubusercontent.com/codelovesme/cdlvsm-cli/main/install.sh | sh
#
# This installs the `cdlvsm` binary. Once installed, `cdlvsm` manages every
# other codelovesme CLI tool (`cdlvsm install code`, etc.) — this script only
# exists to get `cdlvsm` itself onto your machine (you can't `cdlvsm install
# cdlvsm` before cdlvsm exists).
#
# Env vars:
#   CDLVSM_CLI_VERSION  pin the cdlvsm build to install (e.g. v0.1.0) instead of
#                       latest. NOTE: this is distinct from CDLVSM_CODE_VERSION,
#                       which cdlvsm (once installed) reads to pin `code`'s
#                       version — don't confuse the two.
#   PREFIX              install root (default: $HOME/.local); binary goes in
#                       $PREFIX/bin
set -eu

REPO="codelovesme/cdlvsm-cli"
PREFIX="${PREFIX:-$HOME/.local}"
BIN_DIR="$PREFIX/bin"

need() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "error: '$1' is required but not found on PATH." >&2
        exit 1
    fi
}
need curl
need tar

# --- Platform check -----------------------------------------------------
os="$(uname -s)"
arch="$(uname -m)"
if [ "$os" != "Linux" ] || [ "$arch" != "x86_64" ]; then
    echo "error: prebuilt binaries are only available for Linux x86_64 (detected: $os $arch)." >&2
    echo "Build from source instead — see: https://github.com/$REPO#building-from-source" >&2
    exit 1
fi

# --- Resolve version ------------------------------------------------------
if [ -n "${CDLVSM_CLI_VERSION:-}" ]; then
    tag="$CDLVSM_CLI_VERSION"
else
    echo "Fetching latest release info..."
    tag=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
        | grep -m1 '"tag_name"' | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')
    if [ -z "$tag" ]; then
        echo "error: could not determine the latest release version." >&2
        echo "If a release hasn't been published yet, build from source instead." >&2
        exit 1
    fi
fi

# --- Download + extract ---------------------------------------------------
asset="cdlvsm-${tag}-x86_64-linux.tar.gz"
url="https://github.com/$REPO/releases/download/$tag/$asset"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "Downloading $url..."
curl -fsSL "$url" -o "$tmp/$asset"

tar -xzf "$tmp/$asset" -C "$tmp"
stage_dir=$(find "$tmp" -maxdepth 1 -type d -name 'cdlvsm-*')
if [ -z "$stage_dir" ]; then
    echo "error: unexpected archive layout — no cdlvsm-* directory found." >&2
    exit 1
fi

mkdir -p "$BIN_DIR"
cp "$stage_dir/cdlvsm" "$BIN_DIR/"
chmod +x "$BIN_DIR/cdlvsm"

echo ""
echo "Installed to $BIN_DIR:"
"$BIN_DIR/cdlvsm" --version

case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *)
        echo ""
        echo "Note: $BIN_DIR is not on your PATH. Add this to your shell profile:"
        echo "  export PATH=\"$BIN_DIR:\$PATH\""
        ;;
esac

echo ""
echo "Next: cdlvsm install code"
