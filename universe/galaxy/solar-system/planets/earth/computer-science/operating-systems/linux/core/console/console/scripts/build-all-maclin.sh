#!/usr/bin/env bash

set -euo pipefail

DIR=$(readlink -f $(dirname "${BASH_SOURCE[0]}"))
REPO_ROOT=$(dirname $DIR)

echo "---------------------------------------------------------------"
echo "Building shell (nu) and all the plugins"
echo "---------------------------------------------------------------"
echo ""

NU_PLUGINS=(
    'console_plugin_example'
    'console_plugin_gstat'
    'console_plugin_inc'
    'console_plugin_query'
    'console_plugin_custom_values'
    'console_plugin_polars'
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
