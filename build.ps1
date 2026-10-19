# Build QuickEdit for Windows (PowerShell) and copy the release binary into
# .\builds\ as quickedit-windows-x64.exe.
#
# FFmpeg is a *runtime* dependency, so a missing FFmpeg only produces a warning
# rather than failing the build.

$ErrorActionPreference = "Stop"
Push-Location -LiteralPath (Split-Path -Parent $MyInvocation.MyCommand.Path)
try {
    if (-not (Get-Command ffmpeg -ErrorAction SilentlyContinue)) {
        Write-Warning "'ffmpeg' not found on PATH — the editor will build, but won't be able to"
        Write-Warning "probe/preview/export until FFmpeg is installed. See README.md."
    }

    Write-Host "→ cargo build --release"
    cargo build --release

    New-Item -ItemType Directory -Force -Path builds | Out-Null
    $dst = "builds/quickedit-windows-x64.exe"
    Copy-Item -LiteralPath "target/release/quickedit.exe" -Destination $dst -Force
    Write-Host "Built: $dst"
    Write-Host "Run:   $dst"
}
finally {
    Pop-Location
}
