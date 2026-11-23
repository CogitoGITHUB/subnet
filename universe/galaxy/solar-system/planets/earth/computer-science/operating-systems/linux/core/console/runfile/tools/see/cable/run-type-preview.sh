#!/usr/bin/env bash
# run-type-preview-fit.sh: preview for the type list: a starter page, or how to make a new type.
# Called by: cable/run-types.toml. Argument: the selected entry. The how-to is built from the code.
d=$(cd "$(dirname "$0")" && pwd)
ns=$(cd "$d/../../.." && pwd)
export CONSOLE_RUNFILE_ROOT=$ns
w=$(bash "$d/../preview-fit.sh" --width)
if [ "$1" != "new-type" ]; then
  bash "$d/../preview-fit.sh" "/types/$1.run" --contracts
  exit 0
fi
t="/filetypes/types"
{
  echo "NEW FILE TYPE"
  echo "Enter asks for a name, writes the module from the template, and runs run update. Then set the description and the allowed blocks in the new file."
  echo
  echo "== WHERE"
  echo "filetypes/types/NAME.rkt"
  echo
  echo "== THE TYPE RECORD (filetypes/type.rkt)"
  grep -n -E "struct|^\(define|^\(provide" "$t/../type.rkt" | head -12
  echo
  echo "== BLOCKS YOU MAY LIST (parser.rs)"
  awk "/BLOCK_NAMES/{p=1} p{print} p&&/];/{exit}" "/parser/parser.rs"
  echo
  echo "== TYPES THAT EXIST NOW"
  grep -h "file-type \"" "$t"/*.rkt "$t"/*/type.rkt 2>/dev/null
  echo
  s=$(grep -l "(list " "$t"/[!_]*.rkt 2>/dev/null | xargs ls -S 2>/dev/null | tail -1)
  echo "== EXAMPLE: ${s##*/}"
  cat "$s"
} | fold -s -w "$w"
