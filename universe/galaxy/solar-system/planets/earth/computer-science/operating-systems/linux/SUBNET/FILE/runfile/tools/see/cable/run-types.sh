#!/usr/bin/env bash
# run-types.sh: prints the file type names, one per line, then the entry new-type, for the tv list.
# Called by: cable/run-types.toml. Reads the starter pages in the types folder.
d=$(cd "$(dirname "$0")" && pwd)
for f in "$d"/../../../types/*.run; do b=${f##*/}; echo "${b%.run}"; done
echo "new-type"
