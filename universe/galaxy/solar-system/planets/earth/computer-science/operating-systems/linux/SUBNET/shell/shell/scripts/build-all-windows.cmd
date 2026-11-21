@echo off
echo -------------------------------------------------------------------
echo Building shell (nu.exe) and all the plugins
echo -------------------------------------------------------------------
echo.

echo Building shell.exe
cargo build --locked
echo.

call :build crates\shell_plugin_example shell_plugin_example.exe
call :build ..\..\crates\shell_plugin_gstat shell_plugin_gstat.exe
call :build ..\..\crates\shell_plugin_inc shell_plugin_inc.exe
call :build ..\..\crates\shell_plugin_query shell_plugin_query.exe
call :build ..\..\crates\shell_plugin_custom_values shell_plugin_custom_values.exe

cd ..\..
exit /b 0

:build
    setlocal
    set "location=%~1"
    set "target=%~2"

    cd "%location%"
    echo Building %target%
    cargo build
    echo.
    endlocal
exit /b 0
