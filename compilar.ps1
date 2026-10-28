$ErrorActionPreference = "Stop"
Write-Host "WinCache Cleaner - compilacao para Windows" -ForegroundColor Cyan
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Host "Rust nao encontrado. Instale em https://rustup.rs/" -ForegroundColor Red
    exit 1
}
cargo test
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
cargo build --release
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Write-Host "Executavel criado em: target\release\wincache-cleaner.exe" -ForegroundColor Green
