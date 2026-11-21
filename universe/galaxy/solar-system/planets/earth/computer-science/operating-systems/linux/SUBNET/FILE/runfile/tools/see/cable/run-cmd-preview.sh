#!/usr/bin/env bash
# run-cmd-preview-fit.sh: prints usage and description of one run command for the tv preview pane.
# Called by: cable/run-commands.toml. Argument: the selected line of run-commands.txt.
d=$(cd "$(dirname "$0")" && pwd)
w=$(bash "$d/../preview-fit.sh" --width)
IFS=";" read -r name usage desc <<< "$1"
printf "%s\n\nUsage:\n  %s\n\n%s\n" "$name" "$usage" "$desc" | fold -s -w "$w"
