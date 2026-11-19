# Builds the Windows archive.
#
# Result (under target/windows/, not committed):
#   mimizan-lab-xtrans-<version>-x86_64-windows.zip
#     xtrans2dng.exe       the command
#     xtrans2dng-gui.exe   the window; a double click opens no console
#     README.md, LICENSE, third-party/
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

if (-not $SkipBuild) {
    Write-Host "==> cargo build --profile dist --workspace"
    Push-Location $ProjectDir
    try {
        cargo build --profile dist --workspace
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    } finally {
        Pop-Location
    }
}

foreach ($binary in @($Cli, $Gui)) {
    if (-not (Test-Path -LiteralPath $binary)) {
        throw "ERROR: $binary missing"
    }
    Write-Host "==> binary: $binary"
}
& "$PSScriptRoot\check_windows_exe.ps1" -Path $Cli -Kind console
& "$PSScriptRoot\check_windows_exe.ps1" -Path $Gui -Kind gui

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
Copy-Item -LiteralPath $Cli, $Gui -Destination $Root
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
