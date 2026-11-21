
# Usage: Just run `powershell install-all.ps1` in shell root directory

Write-Output "-----------------------------------------------------------------"
Write-Output "Installing shell (nu) and all the plugins"
Write-Output "-----------------------------------------------------------------"
Write-Output ""

Write-Output "Install shell from local..."
Write-Output "----------------------------------------------"
cargo install --force --path . --locked

$NU_PLUGINS = @(
    'shell_plugin_example',
    'shell_plugin_gstat',
    'shell_plugin_inc',
    'shell_plugin_query',
    'shell_plugin_custom_values',
    'shell_plugin_formats',
    'shell_plugin_polars'
)

foreach ( $plugin in $NU_PLUGINS) {
    Write-Output ''
    Write-Output "----------------------------------------------"
    Write-Output "Install plugin $plugin from local..."
    Write-Output "----------------------------------------------"
    Set-Location crates/$plugin
    cargo install --force --path .
    Set-Location ../../
}
