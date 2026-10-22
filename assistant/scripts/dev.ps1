# Inicia Jarvis en modo desarrollo: servicio de voz en otra ventana + app Tauri.
# Uso: powershell -ExecutionPolicy Bypass -File scripts\dev.ps1 [-NoVoice]

param([switch]$NoVoice)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

if (-not $NoVoice) {
    $voice = Join-Path $PSScriptRoot 'start-voice.ps1'
    if (Test-Path (Join-Path $root 'services\voice-service\.venv\Scripts\python.exe')) {
        Start-Process powershell -ArgumentList '-NoExit', '-ExecutionPolicy', 'Bypass', '-File', "`"$voice`"" -WindowStyle Minimized
        Write-Host 'Servicio de voz iniciado en otra ventana (minimizada).'
    } else {
        Write-Host 'Sin entorno de voz: Jarvis funcionara solo por texto. (scripts\setup.ps1 lo crea)' -ForegroundColor Yellow
    }
}

# Tauri arranca Vite (puerto 1420) y compila el backend Rust. La primera vez tarda varios minutos.
npm run tauri:dev -w apps/desktop
