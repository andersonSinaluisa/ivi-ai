# Instala openWakeWord y descarga el modelo preentrenado "hey jarvis" (opcional).
# AVISO: ese modelo es solo para ingles y su licencia es CC BY-NC-SA 4.0 (uso no comercial).
# Detalles y como entrenar "Jarvis" en espanol: docs\voice-wake-word.md

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$svc = Join-Path $root 'services\voice-service'
$python = Join-Path $svc '.venv\Scripts\python.exe'
if (-not (Test-Path $python)) { throw 'Ejecuta primero scripts\setup.ps1' }

& $python -m pip install -e "$svc[wakeword]"
& $python -c "import openwakeword.utils as u; u.download_models(['hey_jarvis']); print('modelo descargado')"

Write-Host "`nPara activarlo: Configuracion > Voz, o en %LOCALAPPDATA%\Jarvis\voice.json pon `"wake_word_enabled`": true y reinicia el servicio." -ForegroundColor Green
Write-Host 'Pruebalo diciendo "Hey Jarvis" (en ingles). No se considera funcional en espanol hasta probarlo con tu voz.'
