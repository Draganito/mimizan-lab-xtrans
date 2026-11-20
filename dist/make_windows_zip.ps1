# Builds the Windows archive.
#
# Result (under target/windows/, not committed):
#   mimizan-lab-xtrans-<version>-x86_64-windows.zip
#     xtrans2dng.exe       the command's starter
#     xtrans2dng-gui.exe   the window's starter; a double click opens no console
#     bin/                 the programs themselves (x86-64-v3)
#     README.md, LICENSE, third-party/
#
# The starters are baseline x86-64. On a CPU without AVX2 and FMA they
# say so and stop. The programs in bin/ would crash there.
#
# The two programs carry the Visual C++ runtime (.cargo/config.toml), and
# the window draws with DirectX 12: nothing has to be installed first.
#
# Usage:  dist/make_windows_zip.ps1 [-SkipBuild]

param(
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

$ProjectDir = Split-Path -Parent $PSScriptRoot
$TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $ProjectDir "target" }
$Cli = Join-Path $TargetDir "dist\xtrans2dng.exe"
$Gui = Join-Path $TargetDir "dist\xtrans2dng-gui.exe"
$LaunchDir = Join-Path $ProjectDir "target\launch\release"
$LaunchCli = Join-Path $LaunchDir "xtrans-launch.exe"
$LaunchGui = Join-Path $LaunchDir "xtrans-launch-gui.exe"

if (-not $SkipBuild) {
    Write-Host "==> cargo build --profile dist --workspace"
    Push-Location $ProjectDir
    try {
        cargo build --profile dist --workspace
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    } finally {
        Pop-Location
    }
    # CARGO_ENCODED_RUSTFLAGS replaces .cargo/config.toml, so the starter
    # does not inherit target-cpu=x86-64-v3. The static C runtime stays.
    Write-Host "==> cargo build --release --manifest-path launch/Cargo.toml (baseline CPU)"
    $savedFlags = $env:CARGO_ENCODED_RUSTFLAGS
    $savedTarget = $env:CARGO_TARGET_DIR
    $env:CARGO_ENCODED_RUSTFLAGS = "-C" + [char]0x1f + "target-feature=+crt-static"
    $env:CARGO_TARGET_DIR = Join-Path $ProjectDir "target\launch"
    Push-Location $ProjectDir
    try {
        cargo build --release --manifest-path (Join-Path $ProjectDir "launch\Cargo.toml")
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        cargo test --release --manifest-path (Join-Path $ProjectDir "launch\Cargo.toml")
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    } finally {
        Pop-Location
        if ($null -eq $savedFlags) {
            Remove-Item Env:CARGO_ENCODED_RUSTFLAGS -ErrorAction SilentlyContinue
        } else {
            $env:CARGO_ENCODED_RUSTFLAGS = $savedFlags
        }
        if ($null -eq $savedTarget) {
            Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
        } else {
            $env:CARGO_TARGET_DIR = $savedTarget
        }
    }
}

foreach ($binary in @($Cli, $Gui, $LaunchCli, $LaunchGui)) {
    if (-not (Test-Path -LiteralPath $binary)) {
        throw "ERROR: $binary missing"
    }
    Write-Host "==> binary: $binary"
}
& "$PSScriptRoot\check_windows_exe.ps1" -Path $Cli -Kind console
& "$PSScriptRoot\check_windows_exe.ps1" -Path $Gui -Kind gui
& "$PSScriptRoot\check_windows_exe.ps1" -Path $LaunchCli -Kind console
& "$PSScriptRoot\check_windows_exe.ps1" -Path $LaunchGui -Kind gui

$versionLine = (& $Cli --version | Out-String).Trim()
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$Version = ($versionLine -split "\s+", 2)[1]
if (-not $Version) { throw "ERROR: could not read version from: $versionLine" }
Write-Host "==> version $Version"

# The window has no console, but it answers into a pipe.
$guiLine = (& $Gui --version | Out-String).Trim()
if ($guiLine -ne "xtrans2dng-gui $Version") {
    throw "ERROR: the window says '$guiLine', the command '$versionLine'"
}

$Name = "mimizan-lab-xtrans-$Version-x86_64-windows"
$Stage = Join-Path ([System.IO.Path]::GetTempPath()) ("xtrans-" + [guid]::NewGuid().ToString("n"))
$Root = Join-Path $Stage $Name
New-Item -ItemType Directory -Path $Root -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $Root "bin") -Force | Out-Null
Copy-Item -LiteralPath $LaunchCli -Destination (Join-Path $Root "xtrans2dng.exe")
Copy-Item -LiteralPath $LaunchGui -Destination (Join-Path $Root "xtrans2dng-gui.exe")
Copy-Item -LiteralPath $Cli -Destination (Join-Path $Root "bin\xtrans2dng.exe")
Copy-Item -LiteralPath $Gui -Destination (Join-Path $Root "bin\xtrans2dng-gui.exe")
$staged = (& (Join-Path $Root "xtrans2dng.exe") --version | Out-String).Trim()
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
if ($staged -ne $versionLine) {
    throw "ERROR: the starter says '$staged', the program '$versionLine'"
}
foreach ($doc in @("README.md", "LICENSE")) {
    Copy-Item -LiteralPath (Join-Path $ProjectDir $doc) -Destination $Root
}
Copy-Item -LiteralPath (Join-Path $PSScriptRoot "third-party") -Destination $Root -Recurse

$Out = Join-Path $TargetDir "windows"
New-Item -ItemType Directory -Path $Out -Force | Out-Null
Get-ChildItem -LiteralPath $Out -Filter "mimizan-lab-xtrans-*-x86_64-windows.zip" -ErrorAction SilentlyContinue |
    Remove-Item -Force
$Zip = Join-Path $Out "$Name.zip"
tar.exe -C $Stage -a -c -f $Zip $Name
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Remove-Item -LiteralPath $Stage -Recurse -Force
Write-Host "==> $Zip"
Get-Item -LiteralPath $Zip | Select-Object FullName, Length
