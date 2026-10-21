# Offline, quota-free Rust verification on Windows.
param(
    [switch]$Full
)
$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Push-Location $repoRoot
try {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        throw "Rust is missing. Install a Rust toolchain first."
    }

    Write-Host "Checking Rust formatting..."
    & cargo fmt --all -- --check
    if ($LASTEXITCODE -ne 0) { throw "Rust formatting check failed." }

    Write-Host "Testing vector shaping..."
    & cargo test --locked -p tracedraw-core shaping::tests
    if ($LASTEXITCODE -ne 0) { throw "Vector shaping tests failed." }

    Write-Host "Checking core lints..."
    & cargo clippy --locked -p tracedraw-core -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw "Core clippy check failed." }

    if ($Full) {
        Write-Host "Running all workspace tests..."
        & cargo test --workspace --locked
        if ($LASTEXITCODE -ne 0) { throw "Workspace tests failed." }
    }

    Write-Host "Local checks passed."
}
finally {
    Pop-Location
}
