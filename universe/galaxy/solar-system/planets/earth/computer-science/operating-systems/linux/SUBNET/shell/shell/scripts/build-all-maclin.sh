#!/usr/bin/env bash

set -euo pipefail

DIR=$(readlink -f $(dirname "${BASH_SOURCE[0]}"))
REPO_ROOT=$(dirname $DIR)

echo "---------------------------------------------------------------"
echo "Building shell (nu) and all the plugins"
echo "---------------------------------------------------------------"
echo ""

NU_PLUGINS=(
    'shell_plugin_example'
    'shell_plugin_gstat'
    'shell_plugin_inc'
    'shell_plugin_query'
    'shell_plugin_custom_values'
    'shell_plugin_polars'
)

echo "Building shell"
(
    cd $REPO_ROOT
    cargo build --locked
)

for plugin in "${NU_PLUGINS[@]}"
do
    echo "Building $plugin..."
    echo "-----------------------------"
    (
        cd "$REPO_ROOT/crates/$plugin"
        cargo build
    )
done
