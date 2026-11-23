#!/usr/bin/env bash
# preview-fit.sh: prints one .run page formatted for the finder pane through the Rust view command; --width prints the pane width.
# Called by: cable/*.toml, cable/*.sh, pick.sh. Usage: preview-fit.sh FILE [--contracts] | preview-fit.sh --width
c=$(stty size </dev/tty 2>/dev/null); c=${c#* }
case $c in ""|*[!0-9]*) c=56 ;; esac
[ -n "$CONSOLE_PANE_PCT" ] && c=$((c * CONSOLE_PANE_PCT / 100))
cfg="${XDG_CONFIG_HOME:-$HOME/.config}/finder/config.toml"
lay=${CONSOLE_LAYOUT:-}; pct=""; sec=""
if [ -f "$cfg" ]; then
  while IFS= read -r l; do
    l=${l#"${l%%[![:space:]]*}"}
    case $l in
      "["*) sec=${l%%]*}"]" ;;
      layout*=*) [ -z "$CONSOLE_LAYOUT" ] && [ "$sec" = "[ui]" ] && lay=${l#*=} ;;
      size*=*) [ "$sec" = "[ui.preview_panel]" ] && pct=${l#*=} ;;
    esac
  done <"$cfg"
fi
lay=${lay//[^a-z]/}; pct=${pct//[^0-9]/}
[ -z "$lay" ] && lay=landscape
[ -z "$pct" ] && pct=50
w=$((c - 3)); [ "$w" -lt 30 ] && w=30
if [ "$1" = "--width" ]; then echo "$w"; exit 0; fi
[ -f "$1" ] || { echo "no such file: $1"; exit 0; }
here=$(cd "${0%/*}" && pwd)
bin="$here/../../runfile/target/debug/runfile"
[ -x "$RUNFILE_BIN" ] && bin="$RUNFILE_BIN"
f=$1; shift; x=""
for a in "$@"; do [ "$a" = "--contracts" ] && x="--contracts"; done
exec "$bin" view "$f" --width "$w" $x
