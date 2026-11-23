#!/usr/bin/env bash

set -euo pipefail

echo ''
echo "----------------------------------------------"
echo "Uninstall nu and all plugins from cargo/bin..."
echo "----------------------------------------------"

NU_PLUGINS=(
    'console_plugin_inc'
    'console_plugin_gstat'
    'console_plugin_query'
    'console_plugin_example'
    'console_plugin_formats'
    'console_plugin_custom_values'
    'console_plugin_polars'
)

cargo uninstall nu
for plugin in "${NU_PLUGINS[@]}"
do
    cargo uninstall "$plugin"
done
