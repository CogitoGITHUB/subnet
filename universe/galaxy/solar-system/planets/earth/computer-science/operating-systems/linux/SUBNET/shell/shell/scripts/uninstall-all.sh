#!/usr/bin/env bash

set -euo pipefail

echo ''
echo "----------------------------------------------"
echo "Uninstall nu and all plugins from cargo/bin..."
echo "----------------------------------------------"

NU_PLUGINS=(
    'shell_plugin_inc'
    'shell_plugin_gstat'
    'shell_plugin_query'
    'shell_plugin_example'
    'shell_plugin_formats'
    'shell_plugin_custom_values'
    'shell_plugin_polars'
)

cargo uninstall nu
for plugin in "${NU_PLUGINS[@]}"
do
    cargo uninstall "$plugin"
done
