# Instala OpenVoice V2 + MeloTTS en el entorno de voz (opcional; ocupa 3-5 GB con PyTorch).
# Uso: powershell -ExecutionPolicy Bypass -File scripts\install-openvoice.ps1 [-Cuda]
# Fuente de los pasos: https://huggingface.co/myshell-ai/OpenVoiceV2 (licencia MIT).

param([switch]$Cuda)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$svc = Join-Path $root 'services\voice-service'
$python = Join-Path $svc '.venv\Scripts\python.exe'
if (-not (Test-Path $python)) { throw 'Ejecuta primero scripts\setup.ps1' }

$freeGb = [math]::Round((Get-Item $root).PSDrive.Free / 1GB, 1)
if ($freeGb -lt 8) { throw "Hacen falta ~8 GB libres y hay $freeGb GB. Libera espacio antes de instalar OpenVoice." }
if (-not (Get-Command git -ErrorAction SilentlyContinue)) { throw 'Necesitas git en el PATH.' }

Write-Host '[1/4] PyTorch' -ForegroundColor Cyan
if ($Cuda) {
    & $python -m pip install torch torchaudio --index-url https://download.pytorch.org/whl/cu121
} else {
    & $python -m pip install torch torchaudio --index-url https://download.pytorch.org/whl/cpu
}

Write-Host '[2/4] OpenVoice' -ForegroundColor Cyan
$vendor = Join-Path $svc 'vendor'
New-Item -ItemType Directory -Force $vendor | Out-Null
$repo = Join-Path $vendor 'OpenVoice'
if (-not (Test-Path $repo)) { git clone --depth 1 https://github.com/myshell-ai/OpenVoice.git $repo }
& $python -m pip install -e $repo

Write-Host '[3/4] MeloTTS' -ForegroundColor Cyan
& $python -m pip install git+https://github.com/myshell-ai/MeloTTS.git
& $python -m unidic download

Write-Host '[4/4] Checkpoints V2' -ForegroundColor Cyan
$models = Join-Path $svc 'models\openvoice'
New-Item -ItemType Directory -Force $models | Out-Null
$zip = Join-Path $models 'checkpoints_v2_0417.zip'
# URL publicada en la documentacion de OpenVoice V2. Verificala alli si la descarga falla.
Invoke-WebRequest -Uri 'https://myshell-public-repo-host.s3.amazonaws.com/openvoice/checkpoints_v2_0417.zip' -OutFile $zip
Expand-Archive -Path $zip -DestinationPath $models -Force
Remove-Item $zip

Write-Host "`nOpenVoice instalado. En Jarvis: Configuracion > Voz > Motor de voz = OpenVoice V2, y crea/activa una voz." -ForegroundColor Green
