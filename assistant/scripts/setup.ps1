# Instala las dependencias de Jarvis (frontend, Rust se descarga al compilar, servicio de voz).
# Uso: powershell -ExecutionPolicy Bypass -File scripts\setup.ps1 [-SkipVoice] [-WithWakeWord]

param(
    [switch]$SkipVoice,
    [switch]$WithWakeWord
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$freeGb = [math]::Round((Get-Item $root).PSDrive.Free / 1GB, 1)
if ($freeGb -lt 5) {
    Write-Host "Solo hay $freeGb GB libres. La primera compilacion de Rust ocupa ~2 GB y el entorno de voz ~1 GB." -ForegroundColor Yellow
    $answer = Read-Host 'Continuar de todas formas? (s/N)'
    if ($answer -ne 's') { exit 1 }
}

Write-Host "`n[1/2] Dependencias de la interfaz (npm)" -ForegroundColor Cyan
npm install
if ($LASTEXITCODE -ne 0) { throw 'npm install fallo' }

if ($SkipVoice) {
    Write-Host 'Servicio de voz omitido (-SkipVoice).'
    exit 0
}

Write-Host "`n[2/2] Servicio de voz (Python 3.11 o 3.12)" -ForegroundColor Cyan
$svc = Join-Path $root 'services\voice-service'
$venv = Join-Path $svc '.venv'
if (-not (Test-Path (Join-Path $venv 'Scripts\python.exe'))) {
    . (Join-Path $PSScriptRoot 'find-python.ps1')
    $found = Find-JarvisPython
    if (-not $found) {
        throw "No se encontro Python 3.11 ni 3.12 (se busco: py -3.11, py -3.12, python3.11, python3.12, python). Instalalo con:  winget install -e --id Python.Python.3.11   y abre una terminal nueva."
    }
    Write-Host "Usando $($found.Version) ($($found.Display))"
    & $found.Exe @($found.Args + @('-m', 'venv', $venv))
    if ($LASTEXITCODE -ne 0) { throw "No se pudo crear el entorno virtual con $($found.Display)" }
}
$python = Join-Path $venv 'Scripts\python.exe'
& $python -m pip install --upgrade pip
$extras = 'audio,dev'
if ($WithWakeWord) { $extras += ',wakeword' }
& $python -m pip install -e "$svc[$extras]"
if ($LASTEXITCODE -ne 0) { throw 'pip install fallo' }

Write-Host "`nListo." -ForegroundColor Green
Write-Host 'La transcripcion y la voz estan en tu servidor: empareja la PC en Configuracion -> Infraestructura.'
Write-Host 'Siguiente: scripts\dev.ps1'
