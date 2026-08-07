#!/bin/sh
set -eu

case "$(uname -s)" in
  Darwin|Linux) ;;
  *) printf '%s\n' "Keel currently supports macOS and Linux." >&2; exit 1 ;;
esac

prefix="${KEEL_INSTALL_PREFIX:-$HOME/.local}"
while [ "$#" -gt 0 ]; do
  case "$1" in
    --prefix) prefix="$2"; shift 2 ;;
    *) printf 'unknown option: %s\n' "$1" >&2; exit 1 ;;
  esac
done

command -v cargo >/dev/null 2>&1 || {
  printf '%s\n' "Rust/Cargo is required to install this source checkout." >&2
  exit 1
}

repo_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$repo_dir"
cargo build --release --locked -p keel-cli
mkdir -p "$prefix/bin"
install -m 0755 target/release/keel "$prefix/bin/keel"
"$prefix/bin/keel" --version
printf 'installed: %s\n' "$prefix/bin/keel"
