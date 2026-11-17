# Build and package zalo-next locally on Windows (Portable .exe + Inno Setup .exe)
# without mutating tracked packaging templates (which keep the 0.0.0 sentinel).
$ErrorActionPreference = "Stop"

$RootDir = Split-Path -Parent $PSScriptRoot
Push-Location $RootDir
try {
    $MetaJson = cargo metadata --locked --no-deps --format-version 1 | ConvertFrom-Json
    $Pkg = $MetaJson.packages | Where-Object { $_.name -eq "zalo-next" } | Select-Object -First 1
    $Ver = $Pkg.version
    if (-not $Ver) {
        throw "Could not resolve zalo-next version from Cargo.toml"
    }

    Write-Host "=== Building zalo-next v$Ver ==="
    cargo build --release --locked --bin zalo-next

    New-Item -ItemType Directory -Force -Path dist | Out-Null
    $PortableOut = "dist\zalo-next-$Ver-portable-windows-x86_64.exe"
    Copy-Item "target\release\zalo-next.exe" $PortableOut -Force
    Write-Host "=== Built $PortableOut ==="

    if (Get-Command iscc -ErrorAction SilentlyContinue) {
        iscc "/DMyAppVersion=$Ver" "packaging\windows\installer.iss"
        $SetupOut = "dist\zalo-next-$Ver-setup-windows-x86_64.exe"
        Move-Item "dist\zalo-next-setup-windows-x86_64.exe" $SetupOut -Force
        Write-Host "=== Built $SetupOut ==="
    } else {
        Write-Warning "Inno Setup (iscc) not found in PATH; skipped setup installer generation."
    }
} finally {
    Pop-Location
}
