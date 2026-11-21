#!/bin/sh
SELF=$(cd "$(dirname "$0")" && pwd)
DIR=$(dirname "$SELF")
PREFIX=$(dirname "$(dirname "$DIR")")
CLI="$DIR/text-editor/editor/entry/cli.rkt"
RACKET="${SUBNET_RACKET_BIN:-$HOME/opt/racket-full/bin/racket}"
RUNFILE="$PREFIX/runfile/current/bin/runfile"
if [ -z "$RUNFILE_BIN" ] && [ -x "$RUNFILE" ]; then RUNFILE_BIN="$RUNFILE"; export RUNFILE_BIN; fi
if [ -z "$SUBNET_HOT_PATH_BIN" ]; then SUBNET_HOT_PATH_BIN="$DIR/bin/hot-path"; export SUBNET_HOT_PATH_BIN; fi
export PLTCOMPILEDROOTS="$HOME/run-compiled" SUBNET_ROOT="$DIR" LANG=C.UTF-8 LC_ALL=C.UTF-8
case "$1" in
update|test|docs|trace|record|record-module) exec "$RACKET" "$CLI" "$@" ;;
esac
if [ -z "$1" ]; then exec "$RACKET" "$CLI" edit; fi
exec "$RACKET" "$CLI" edit "$1"
