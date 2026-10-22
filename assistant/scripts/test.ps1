# Ejecuta todas las pruebas: Rust, interfaz y servicio de voz.
# Pruebas de integracion reales (opcionales):
#   $env:JARVIS_TEST_OLLAMA = '1'   -> prueba contra Ollama en 127.0.0.1:11434 (modelo: $env:JARVIS_TEST_MODEL, por defecto qwen3:4b)
#   $env:JARVIS_TEST_WHISPER = '1'  -> transcribe con el modelo whisper "tiny" real

$root = Split-Path -Parent $PSScriptRoot
$results = [ordered]@{}

Set-Location (Join-Path $root 'apps\desktop\src-tauri')
cargo test -- --nocapture
$results['Rust (cargo test)'] = $LASTEXITCODE

Set-Location $root
npm run test -w apps/desktop
$results['Interfaz (vitest)'] = $LASTEXITCODE
npm run typecheck -w apps/desktop
$results['Interfaz (tsc)'] = $LASTEXITCODE

$python = Join-Path $root 'services\voice-service\.venv\Scripts\python.exe'
if (Test-Path $python) {
    Set-Location (Join-Path $root 'services\voice-service')
    & $python -m pytest
    $results['Voz (pytest)'] = $LASTEXITCODE
} else {
    $results['Voz (pytest)'] = 'sin entorno (scripts\setup.ps1)'
}

Set-Location $root
Write-Host "`nResumen" -ForegroundColor Cyan
foreach ($k in $results.Keys) {
    $v = $results[$k]
    $ok = $v -eq 0
    $text = if ($ok) { 'OK' } elseif ($v -is [int]) { "FALLO ($v)" } else { $v }
    Write-Host ("{0,-22} {1}" -f $k, $text) -ForegroundColor $(if ($ok) { 'Green' } else { 'Yellow' })
}
