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
# keel-shim ships ALONGSIDE keel: the parent runtime execs it for command
# interposition and resolves it next to its own binary (a missing shim is a
# broken install, fail-closed — not a degraded mode).
cargo build --release --locked -p keel-cli -p keel-shim
mkdir -p "$prefix/bin"
# Stage then rename — NEVER write in place over the destination. On macOS
# overwriting a Mach-O that a live process has mapped does not fail with
# ETXTBSY: it silently invalidates the signature the kernel cached for that
# vnode, and every later exec of the path dies with SIGKILL (CODESIGNING /
# "Taskgated Invalid Signature") — an upgrade during a running `keel claude`
# would brick the binary. `mv` within the same directory is rename(2): atomic,
# lands on a fresh inode, and REPLACES a symlinked destination instead of
# following it through onto the target's inode.
for bin in keel keel-shim; do
  install -m 0755 "target/release/$bin" "$prefix/bin/.$bin.new"
  mv -f "$prefix/bin/.$bin.new" "$prefix/bin/$bin"
done
"$prefix/bin/keel" --version
printf 'installed: %s\n' "$prefix/bin/keel"
printf 'installed: %s\n' "$prefix/bin/keel-shim"
