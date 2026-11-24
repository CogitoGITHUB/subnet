#!/usr/bin/env bash
set -eu
export LC_ALL=C.UTF-8
ROOT="$(cd "$(dirname "$0")" && pwd)"
OPT="${CONSOLE_OPT:-$HOME/opt}"
WORK="${CONSOLE_WORK:-$HOME/run-install-work}"
RACKET_VERSION="${CONSOLE_RACKET_VERSION:-9.3}"
RUST_VERSION="${CONSOLE_RUST_VERSION:-1.99.0}"
JOBS="${CONSOLE_JOBS:-2}"
DRY="${CONSOLE_INSTALL_DRY:-}"
RACKET_DIR="$OPT/racket-full"
CARGO_DIR="$HOME/.cargo"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"

say() { printf "== %s\n" "$*"; }
die() { printf "FAILED: %s\n" "$*" >&2; exit 1; }
run() { if [ -n "$DRY" ]; then printf "would run: %s\n" "$*"; else "$@"; fi; }
inside() { d="$1"; shift; if [ -n "$DRY" ]; then printf "would run in %s: %s\n" "$d" "$*"; else ( cd "$d" && "$@" ); fi; }
fetch() {
  if [ -n "$DRY" ]; then printf "would download: %s\n" "$1"; return 0; fi
  if command -v curl >/dev/null 2>&1; then curl -fL --retry 3 -o "$2" "$1"
  elif command -v wget >/dev/null 2>&1; then wget -O "$2" "$1"
  else die "need curl or wget"; fi
}

[ "$(uname -o 2>/dev/null || true)" = "Android" ] && die "Android libc is not supported: use a proot distro or a Linux distro"
case "$(uname -m)" in
  aarch64|arm64) RUST_ARCH=aarch64 ;;
  x86_64|amd64) RUST_ARCH=x86_64 ;;
  *) die "unsupported machine type $(uname -m)" ;;
esac

if [ -f /etc/alpine-release ]; then
  say "alpine: installing build tools and Rust with apk"
  run apk add --no-cache bash build-base curl xz tar gzip git linux-headers openssl-dev zlib-dev sqlite-dev ncurses-dev libffi-dev cargo rust
fi

say "checking tools"
missing=""
for t in cc make tar xz gzip; do command -v "$t" >/dev/null 2>&1 || missing="$missing $t"; done
command -v curl >/dev/null 2>&1 || command -v wget >/dev/null 2>&1 || missing="$missing curl-or-wget"
[ -z "$missing" ] || die "missing tools:$missing (install them from source or with your system, then run again)"
for h in openssl/ssl.h zlib.h sqlite3.h; do
  found=0
  for d in /usr/include /usr/local/include /usr/include/*-linux-gnu; do [ -f "$d/$h" ] && found=1; done
  [ "$found" = 1 ] || say "warning: header $h not found, some Rust crates may fail to build"
done

run mkdir -p "$WORK"

if [ -x "$RACKET_DIR/bin/racket" ]; then
  say "racket present at $RACKET_DIR"
else
  say "racket: building $RACKET_VERSION from source (long)"
  fetch "https://download.racket-lang.org/installers/$RACKET_VERSION/racket-minimal-$RACKET_VERSION-src.tgz" "$WORK/racket-src.tgz"
  run tar -xzf "$WORK/racket-src.tgz" -C "$WORK"
  run mkdir -p "$WORK/racket-$RACKET_VERSION/src/build"
  inside "$WORK/racket-$RACKET_VERSION/src/build" ../configure --prefix="$RACKET_DIR"
  inside "$WORK/racket-$RACKET_VERSION/src/build" make -j "$JOBS"
  inside "$WORK/racket-$RACKET_VERSION/src/build" make install
fi

if [ -x "$CARGO_DIR/bin/cargo" ] || command -v cargo >/dev/null 2>&1; then
  say "cargo present"
else
  say "rust: installing $RUST_VERSION from the official tarball"
  NAME="rust-$RUST_VERSION-$RUST_ARCH-unknown-linux-gnu"
  fetch "https://static.rust-lang.org/dist/$NAME.tar.xz" "$WORK/rust.tar.xz"
  run tar -xJf "$WORK/rust.tar.xz" -C "$WORK"
  inside "$WORK/$NAME" ./install.sh --prefix="$CARGO_DIR" --disable-ldconfig
fi

export PATH="$CARGO_DIR/bin:$RACKET_DIR/bin:$PATH"
export PLTCOMPILEDROOTS="$HOME/run-compiled"

export CONSOLE_ECOSYSTEM_ROOT="$ROOT/console"
SUB="$ROOT/console/entry/entry/subnet.rkt"
say "compiling the console entry (first run is slow)"
say "compiling the console entry, installing packages on demand"
for i in $(seq 1 60); do
  if out=$("$RACKET_DIR/bin/raco" make -j "$JOBS" "$SUB" 2>&1); then
    printf "%s\n" "$out" | tail -3; break
  fi
  c=$(printf "%s\n" "$out" | grep "collection:" | head -1 | cut -d "\"" -f2)
  [ -n "$c" ] || { printf "%s\n" "$out" | tail -15; die "compile failed, no missing collection"; }
  say "missing collection: $c, installing"
  "$RACKET_DIR/bin/raco" pkg install --auto --skip-installed --no-docs "$c" \
    || "$RACKET_DIR/bin/raco" pkg install --auto --skip-installed --no-docs "$c-lib" \
    || die "cannot install $c"
done
say "console installs everything"
run "$RACKET_DIR/bin/racket" "$SUB" install
say "done. $WORK holds the downloaded sources and can be deleted"
