#!/usr/bin/env bash

set -euo pipefail

DIR=$(readlink -f $(dirname "${BASH_SOURCE[0]}"))
REPO_ROOT=$(dirname $DIR)

echo "-----------------------------------------------------------------"
echo "Installing shell (nu) and all the plugins"
echo "-----------------------------------------------------------------"
echo ""

echo "Install shell from local..."
echo "----------------------------------------------"
cargo install --force --path "$REPO_ROOT" --locked

NU_PLUGINS=(
    'shell_plugin_inc'
    'shell_plugin_gstat'
    'shell_plugin_query'
    'shell_plugin_example'
    'shell_plugin_custom_values'
    'shell_plugin_formats'
    'shell_plugin_polars'
)

for plugin in "${NU_PLUGINS[@]}"
do
    echo ''
    echo "----------------------------------------------"
    echo "Install plugin $plugin from local..."
    echo "----------------------------------------------"
    cargo install --force --path "$REPO_ROOT/crates/$plugin"
done
