# Inspecciona el entorno de desarrollo de Jarvis en Windows.
# Uso: powershell -ExecutionPolicy Bypass -File scripts\check-env.ps1
# Solo lee: no instala ni cambia nada.

$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot
$problems = @()

function Show($name, $ok, $detail) {
    $mark = if ($ok) { '[OK]   ' } else { '[FALTA]' }
    $color = if ($ok) { 'Green' } else { 'Yellow' }
    Write-Host ("{0} {1,-22} {2}" -f $mark, $name, $detail) -ForegroundColor $color
}

function Version-Of($cmd, $args1) {
    $c = Get-Command $cmd -ErrorAction SilentlyContinue
    if (-not $c) { return $null }
    try { return (& $cmd $args1 2>&1 | Select-Object -First 1).ToString().Trim() } catch { return $null }
}

Write-Host "`nJarvis - comprobacion del entorno`n" -ForegroundColor Cyan

# Sistema y disco
$os = Get-CimInstance Win32_OperatingSystem
Show 'Windows' $true "$($os.Caption) $($os.Version)"
$drive = (Get-Item $root).PSDrive
$freeGb = [math]::Round($drive.Free / 1GB, 1)
$diskOk = $freeGb -ge 5
Show "Disco $($drive.Name):" $diskOk "$freeGb GB libres (recomendado: 5 GB para compilar; los modelos estan en el servidor)"
if (-not $diskOk) { $problems += "Libera espacio en $($drive.Name):" }

# Node
$node = Version-Of 'node' '--version'
$nodeOk = $node -and ([version]($node.TrimStart('v').Split('-')[0]) -ge [version]'20.19')
Show 'Node.js' $nodeOk ($(if ($node) { $node } else { 'no instalado (https://nodejs.org, LTS 22)' }))
if (-not $nodeOk) { $problems += 'Instala Node.js 22 LTS.' }
$npm = Version-Of 'npm' '--version'
Show 'npm' ([bool]$npm) ($(if ($npm) { $npm } else { 'no encontrado' }))

# Rust
$rustc = Version-Of 'rustc' '--version'
$rustOk = $false
if ($rustc -match 'rustc (\d+\.\d+)') { $rustOk = [version]$Matches[1] -ge [version]'1.85' }
Show 'Rust' $rustOk ($(if ($rustc) { $rustc } else { 'no instalado (https://rustup.rs)' }))
if (-not $rustOk) { $problems += 'Instala o actualiza Rust (rustup update stable).' }

# Herramientas de compilación de C++ (MSVC) que Tauri necesita
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$msvc = $null
if (Test-Path $vswhere) { $msvc = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property displayName }
Show 'MSVC Build Tools' ([bool]$msvc) ($(if ($msvc) { $msvc } else { 'faltan "Desarrollo de escritorio con C++" de Visual Studio Build Tools' }))
if (-not $msvc) { $problems += 'Instala Visual Studio Build Tools con "Desarrollo para el escritorio con C++".' }

# WebView2
$wv = $null
foreach ($k in @('HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}',
                 'HKCU:\Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}')) {
    if (Test-Path $k) { $wv = (Get-ItemProperty $k).pv }
}
Show 'WebView2' ([bool]$wv) ($(if ($wv) { $wv } else { 'no encontrado (viene con Windows 11; si no, instala Evergreen Runtime)' }))

# Python 3.11
. (Join-Path $PSScriptRoot 'find-python.ps1')
$found = Find-JarvisPython
$pyOk = [bool]$found
Show 'Python 3.11/3.12' $pyOk ($(if ($pyOk) { "$($found.Version) ($($found.Display))" } else { 'no encontrado. Instala con: winget install -e --id Python.Python.3.11' }))
if (-not $pyOk) { $problems += 'Instala Python 3.11 (el servicio de voz lo requiere).' }

# Servidor (Cloud Core): modelos, transcripcion y voz estan alli; en esta PC no corre ningun modelo.
$toml = Join-Path $root 'config\jarvis.toml'
$server = $null
if (Test-Path $toml) {
    $line = Select-String -Path $toml -Pattern '^\s*default_url\s*=\s*"([^"]+)"' | Select-Object -First 1
    if ($line) { $server = $line.Matches[0].Groups[1].Value }
}
if ($server) {
    try {
        $h = Invoke-RestMethod -Uri "$server/api/v1/health" -TimeoutSec 8
        Show 'Servidor' $true "$server responde (protocolo $($h.features.sync), busqueda $($h.features.vectorSearch))"
    } catch {
        Show 'Servidor' $false "$server no responde: $($_.Exception.Message)"
        $problems += "Comprueba la conexion con $server (o cambia [cloud] default_url en config\jarvis.toml)."
    }
} else {
    Show 'Servidor' $false 'sin [cloud] default_url en config\jarvis.toml'
}

# windows-agent
$sid = ([System.Security.Principal.WindowsIdentity]::GetCurrent()).User.Value
$pipe = "\\.\pipe\jarvis-tools-$sid"
$pipeOk = @([System.IO.Directory]::GetFiles('\\.\pipe\')) -contains $pipe
Show 'windows-agent' $pipeOk ($(if ($pipeOk) { "Local Tool API activa ($pipe)" } else { "sin pipe. Opcional: activa [tool_api] enabled = true en %LOCALAPPDATA%\Jarvis\Agent\config.toml y ejecuta jarvis-agent run" }))

# Servicio de voz
$venv = Join-Path $root 'services\voice-service\.venv\Scripts\python.exe'
Show 'Entorno de voz' (Test-Path $venv) ($(if (Test-Path $venv) { 'services\voice-service\.venv listo' } else { 'sin crear: ejecuta scripts\setup.ps1' }))

Write-Host ''
if ($problems.Count) {
    Write-Host 'Pendiente:' -ForegroundColor Yellow
    $problems | ForEach-Object { Write-Host " - $_" }
} else {
    Write-Host 'Todo listo. Siguiente paso: scripts\setup.ps1 y luego scripts\dev.ps1' -ForegroundColor Green
}
