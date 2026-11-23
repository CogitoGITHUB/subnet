# git-tui installer for Windows (PowerShell 5.1 or later).
#
#   irm https://gitoui.nayji7.dev/install.ps1 | iex
#
# Downloads the latest release (or $env:GIT_TUI_VERSION), checks its SHA256
# against checksum.txt, installs git-tui.exe into $env:INSTALL_DIR (default
# %LOCALAPPDATA%\Programs\git-tui) and adds that folder to the user PATH.
#
#   GIT_TUI_VERSION          pin a tag, e.g. v0.4.1 (default: latest)
#   INSTALL_DIR             install folder
#   GIT_TUI_INSTALL_NO_WELCOME  skip the quick-start trailer

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$Repo = 'NayJi7/gitoui'
$Target = 'x86_64-pc-windows-msvc'

function Fail($msg) { Write-Host "  error: $msg" -ForegroundColor Red; throw $msg }

# Only 64-bit Intel/AMD builds are published (Arm Windows runs them emulated).
if (-not [Environment]::Is64BitOperatingSystem) {
    Fail 'no prebuilt git-tui for 32-bit Windows, use: cargo install git-tui'
}

$InstallDir = if ($env:INSTALL_DIR) { $env:INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\git-tui' }

# Which version?
if ($env:GIT_TUI_VERSION) {
    $Tag = if ($env:GIT_TUI_VERSION.StartsWith('v')) { $env:GIT_TUI_VERSION } else { "v$($env:GIT_TUI_VERSION)" }
} else {
    $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -Headers @{ 'User-Agent' = 'git-tui-installer' }
    $Tag = $release.tag_name
}
$Version = $Tag.TrimStart('v')
$Archive = "git-tui-$Version-$Target.zip"
$Base = "https://github.com/$Repo/releases/download/$Tag"

$Exe = Join-Path $InstallDir 'git-tui.exe'
if (Test-Path $Exe) {
    $current = (& $Exe --version 2>$null) -replace '^\S+\s+', ''
    if ($current -eq $Version) { Write-Host "  git-tui $Version is already installed."; return }
}

$Tmp = Join-Path ([IO.Path]::GetTempPath()) "git-tui-install-$([guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $Tmp | Out-Null
try {
    Write-Host "  Downloading git-tui $Version ..."
    Invoke-WebRequest -Uri "$Base/$Archive" -OutFile (Join-Path $Tmp $Archive)
    Invoke-WebRequest -Uri "$Base/checksum.txt" -OutFile (Join-Path $Tmp 'checksum.txt')

    $line = Get-Content (Join-Path $Tmp 'checksum.txt') | Where-Object { $_ -match [regex]::Escape($Archive) } | Select-Object -First 1
    if (-not $line) { Fail "no checksum listed for $Archive, refusing to install." }
    $expected = ($line -split '\s+')[0].ToLower()
    $actual = (Get-FileHash (Join-Path $Tmp $Archive) -Algorithm SHA256).Hash.ToLower()
    if ($expected -ne $actual) { Fail "checksum mismatch for $Archive, refusing to install." }

    Expand-Archive -Path (Join-Path $Tmp $Archive) -DestinationPath $Tmp -Force
    if (-not (Test-Path (Join-Path $Tmp 'git-tui.exe'))) { Fail 'git-tui.exe is missing from the archive.' }

    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    # A running git-tui.exe cannot be overwritten, but it can be renamed:
    # that is how `git-tui --update` replaces itself.
    if (Test-Path $Exe) {
        $old = "$Exe.old"
        if (Test-Path $old) { Remove-Item $old -Force -ErrorAction SilentlyContinue }
        Move-Item $Exe $old -Force
    }
    Copy-Item (Join-Path $Tmp 'git-tui.exe') $Exe -Force
    Remove-Item "$Exe.old" -Force -ErrorAction SilentlyContinue
} finally {
    Remove-Item $Tmp -Recurse -Force -ErrorAction SilentlyContinue
}

# One anonymous ping after a real install, to count installs (no data is
# sent). Set GIT_TUI_NO_TRACK=1 to skip it.
if (-not $env:GIT_TUI_NO_TRACK) {
    try {
        Invoke-WebRequest -Uri 'https://git-tui-install-counter.nayji7.workers.dev/ping' `
            -UserAgent "git-tui-install-windows/$Version" -TimeoutSec 2 -UseBasicParsing | Out-Null
    } catch { }
}

# PATH (user level, effective in new terminals).
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (($userPath -split ';') -notcontains $InstallDir) {
    [Environment]::SetEnvironmentVariable('Path', ($userPath.TrimEnd(';') + ';' + $InstallDir), 'User')
    Write-Host "  Added $InstallDir to your PATH (open a new terminal)."
}

Write-Host "  git-tui $Version installed in $InstallDir" -ForegroundColor Green
if (-not $env:GIT_TUI_INSTALL_NO_WELCOME) {
    Write-Host ''
    Write-Host '  Quick start, inside any git repo:'
    Write-Host '    git-tui        open the commit-graph viewer'
    Write-Host '    git-tui --help all flags and options'
    Write-Host ''
}
