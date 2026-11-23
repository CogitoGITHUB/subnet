@echo off
echo -------------------------------------------------------------------
echo Building shell (nu.exe) and all the plugins
echo -------------------------------------------------------------------
echo.

echo Building shell.exe
cargo build --locked
echo.

call :build crates\console_plugin_example console_plugin_example.exe
call :build ..\..\crates\console_plugin_gstat console_plugin_gstat.exe
call :build ..\..\crates\console_plugin_inc console_plugin_inc.exe
call :build ..\..\crates\console_plugin_query console_plugin_query.exe
call :build ..\..\crates\console_plugin_custom_values console_plugin_custom_values.exe

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
