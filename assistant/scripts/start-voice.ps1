# Arranca el servicio de voz en http://127.0.0.1:8765
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$svc = Join-Path $root 'services\voice-service'
$python = Join-Path $svc '.venv\Scripts\python.exe'
if (-not (Test-Path $python)) {
    Write-Host 'No existe el entorno de voz. Ejecuta primero scripts\setup.ps1' -ForegroundColor Yellow
    exit 1
}
Set-Location $svc
& $python -m app
