<#
.SYNOPSIS
    Escenarios de aceptacion WIN-E2E-01..14 contra Jarvis Cloud real (HU-WIN-053).

.DESCRIPTION
    Registra un device nuevo, levanta un agente aislado (raiz temporal propia), ejecuta los
    escenarios de docs/11-testing/integration-scenarios.md contra el Cloud y el broker reales y
    archiva la evidencia de cada uno en scripts/e2e-evidence/<timestamp>/.

    Lo que este script no hace es fingir: si un escenario no puede ejecutarse en el entorno actual
    (falta una app, el broker no es controlable, el Cloud no despacha ese tipo), se marca SKIP con
    el motivo; nunca PASS.

.EXAMPLE
    pwsh -File scripts/e2e.ps1 -Email demo@example.com -Password '<passphrase>'
#>
[CmdletBinding()]
param(
    [string]$ApiBase = 'http://127.0.0.1:8080',
    [string]$Email = 'demo@example.com',
    [string]$Password = $env:JARVIS_E2E_PASSWORD,
    [string]$BrokerHost = '127.0.0.1',
    [int]$BrokerPort = 1883,
    [string]$MosquittoContainer = 'jarvis-dev-mosquitto-1',
    [string]$AgentExe,
    [string]$SignExe,
    [string]$CloudPasswordFile,
    [switch]$KeepDevice
)

$ErrorActionPreference = 'Stop'
$script:Results = [System.Collections.Generic.List[object]]::new()
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$repoRoot = Split-Path -Parent $PSScriptRoot
$EvidenceDir = Join-Path $repoRoot "scripts\e2e-evidence\$stamp"
$AgentRoot = Join-Path $env:TEMP "jarvis-e2e-$stamp"

if (-not $AgentExe) { $AgentExe = Join-Path $repoRoot 'target\debug\jarvis-agent.exe' }
if (-not $SignExe) { $SignExe = Join-Path $repoRoot 'target\debug\jarvis-sign.exe' }
if (-not $CloudPasswordFile) {
    $CloudPasswordFile = Join-Path (Split-Path -Parent $repoRoot) 'api\.local-secrets\mqtt_cloud_password'
}
if (-not $Password) { throw 'Falta la contrasena del owner: pasa -Password o define JARVIS_E2E_PASSWORD.' }

New-Item -ItemType Directory -Force -Path $EvidenceDir | Out-Null
New-Item -ItemType Directory -Force -Path $AgentRoot | Out-Null

function Write-Step([string]$text) { Write-Host "  $text" -ForegroundColor DarkGray }

function Save-Evidence([string]$name, $data) {
    $data | ConvertTo-Json -Depth 12 | Set-Content -Path (Join-Path $EvidenceDir "$name.json") -Encoding utf8
}

function Add-Result([string]$id, [string]$title, [string]$status, [string]$detail) {
    $color = switch ($status) { 'PASS' { 'Green' } 'FAIL' { 'Red' } default { 'Yellow' } }
    Write-Host ("{0,-12} {1,-6} {2}" -f $id, $status, $detail) -ForegroundColor $color
    $script:Results.Add([pscustomobject]@{ Id = $id; Title = $title; Status = $status; Detail = $detail })
}

function Test-Ok([bool]$condition) { if ($condition) { 'PASS' } else { 'FAIL' } }

# ---- Cloud REST ---------------------------------------------------------------------

function Invoke-Api {
    param([string]$Method, [string]$Path, $Body, [switch]$AllowError)
    $headers = @{}
    if ($script:Token) { $headers['Authorization'] = "Bearer $($script:Token)" }
    $params = @{ Method = $Method; Uri = "$ApiBase$Path"; Headers = $headers; ContentType = 'application/json' }
    if ($null -ne $Body) { $params['Body'] = ($Body | ConvertTo-Json -Depth 10) }
    try {
        return Invoke-RestMethod @params
    } catch {
        if (-not $AllowError) { throw }
        $response = $_.Exception.Response
        $code = 0
        $text = $_.Exception.Message
        if ($response) {
            $code = [int]$response.StatusCode
            try { $text = [System.IO.StreamReader]::new($response.GetResponseStream()).ReadToEnd() } catch {}
        }
        return [pscustomobject]@{ HttpError = $true; StatusCode = $code; Body = $text }
    }
}

function New-Task {
    param([string]$Type, $Payload, [int]$MaxDurationSeconds = 60, [int]$ExpiresInSeconds = 300)
    $body = @{
        targetDeviceId     = $script:DeviceId
        type               = $Type
        payload            = $Payload
        expiresInSeconds   = $ExpiresInSeconds
        maxDurationSeconds = $MaxDurationSeconds
    }
    return Invoke-Api -Method POST -Path '/api/v1/tasks' -Body $body -AllowError
}

# Firma con la herramienta del owner (jarvis-sign). La clave privada nunca sale de esta maquina:
# el Cloud solo transporta la firma y quien la verifica es el agente (ADR-012).
function Invoke-Sign {
    param([string[]]$SignArgs)
    $output = & $SignExe @SignArgs 2>&1
    if ($LASTEXITCODE -ne 0) { throw "jarvis-sign fallo: $output" }
    return ($output -join "`n" | ConvertFrom-Json)
}

function New-SignedTask {
    param([string]$Type, [hashtable]$Payload, [int]$ExpiresInSeconds = 300)
    # El payload va por archivo: PowerShell se come las comillas al pasar JSON a un exe nativo.
    $payloadFile = Join-Path $AgentRoot "payload-$([guid]::NewGuid().ToString('N')).json"
    Set-Content -Path $payloadFile -Value ($Payload | ConvertTo-Json -Compress -Depth 5) -Encoding utf8
    $body = Invoke-Sign @(
        'task', '--key', $script:SignKey, '--target', $script:DeviceId, '--type', $Type,
        '--payload-file', $payloadFile, '--requested-by', "User:$($script:OwnerId)",
        '--expires-in', "$ExpiresInSeconds"
    )
    return @{ Body = $body; PayloadFile = $payloadFile }
}

function Wait-Task {
    param(
        [string]$TaskId,
        [int]$TimeoutSeconds = 60,
        [string[]]$Until = @('Completed', 'Failed', 'Rejected', 'Cancelled', 'Expired')
    )
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    $task = $null
    while ((Get-Date) -lt $deadline) {
        $task = Invoke-Api -Method GET -Path "/api/v1/tasks/$TaskId"
        if ($Until -contains $task.status) { return $task }
        Start-Sleep -Milliseconds 400
    }
    return $task
}

function Wait-Ready {
    # Online no basta: el Cloud no despacha mientras el device siga marcado como ocupado.
    param([int]$TimeoutSeconds = 120)
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        $device = Invoke-Api -Method GET -Path "/api/v1/devices/$($script:DeviceId)"
        if ($device.presenceState -eq 'Online' -and -not $device.busy) {
            Start-Sleep -Seconds 2
            return $device
        }
        Start-Sleep -Milliseconds 500
    }
    return $null
}

function Wait-Device {
    param([string]$PresenceState, [int]$TimeoutSeconds = 60)
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        $device = Invoke-Api -Method GET -Path "/api/v1/devices/$($script:DeviceId)"
        if ($device.presenceState -eq $PresenceState) { return $device }
        Start-Sleep -Milliseconds 500
    }
    return $null
}

# ---- Agente local -------------------------------------------------------------------

function Invoke-Agent {
    param([string[]]$AgentArgs, [string]$StdIn)
    $psi = [System.Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = $AgentExe
    # Windows PowerShell 5.1 no tiene ArgumentList: se cita a mano cada argumento.
    $quoted = @('--root', $AgentRoot) + $AgentArgs | ForEach-Object {
        if ($_ -match '[\s"]') { '"' + ($_ -replace '"', '\"') + '"' } else { $_ }
    }
    $psi.Arguments = $quoted -join ' '
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.RedirectStandardInput = [bool]$StdIn
    $psi.UseShellExecute = $false
    $process = [System.Diagnostics.Process]::Start($psi)
    if ($StdIn) {
        # Bytes crudos: el StreamWriter de PowerShell 5.1 antepone un BOM y el secreto llegaria
        # corrupto al agente.
        $bytes = [System.Text.Encoding]::ASCII.GetBytes($StdIn + "`n")
        $process.StandardInput.BaseStream.Write($bytes, 0, $bytes.Length)
        $process.StandardInput.BaseStream.Flush()
        $process.StandardInput.Close()
    }
    $stdout = $process.StandardOutput.ReadToEnd()
    $stderr = $process.StandardError.ReadToEnd()
    $process.WaitForExit()
    return [pscustomobject]@{ ExitCode = $process.ExitCode; StdOut = $stdout; StdErr = $stderr }
}

function Start-Agent {
    # Un log por arranque: si no, el reinicio de WIN-E2E-10 borraria la evidencia anterior.
    $script:AgentRun = [int]$script:AgentRun + 1
    $script:AgentProcess = Start-Process -FilePath $AgentExe `
        -ArgumentList @('--root', $AgentRoot, 'run', '--console') `
        -RedirectStandardOutput (Join-Path $EvidenceDir "agent-console-$($script:AgentRun).log") `
        -RedirectStandardError (Join-Path $EvidenceDir "agent-console-$($script:AgentRun).err.log") `
        -PassThru -WindowStyle Hidden
    Start-Sleep -Seconds 2
}

function Stop-Agent {
    if ($script:AgentProcess -and -not $script:AgentProcess.HasExited) {
        Invoke-Agent -AgentArgs @('stop') | Out-Null
        if (-not $script:AgentProcess.WaitForExit(8000)) { $script:AgentProcess.Kill() }
    }
}

# ---- Preparacion --------------------------------------------------------------------

Write-Host "`n== Preparacion ==" -ForegroundColor Cyan
Write-Step "Evidencia en $EvidenceDir"
if (-not (Test-Path $AgentExe)) { throw "No existe $AgentExe. Compila con: cargo build -p jarvis-agent" }

$login = Invoke-Api -Method POST -Path '/api/v1/auth/login' -Body @{ email = $Email; password = $Password }
$script:Token = $login.accessToken
$script:OwnerId = $login.user.id
Write-Step "Owner $($login.user.email) ($($script:OwnerId))"

$registration = Invoke-Api -Method POST -Path '/api/v1/devices' -Body @{ name = "jarvis-e2e-$stamp"; type = 'windows' }
$script:DeviceId = $registration.device.id
$secret = $registration.credential.secret
Write-Step "Device $($registration.device.name) ($($script:DeviceId))"
Save-Evidence 'device' $registration.device

# Clave del owner para firmar (WIN-E2E-15/16). La privada se queda aqui; en la politica del agente
# solo entra la publica, que es el ancla de confianza local.
if (-not (Test-Path $SignExe)) { throw "No existe $SignExe. Compila con: cargo build -p jarvis-sign" }
$script:SignKey = Join-Path $AgentRoot 'owner.key'
& $SignExe keygen --key-id 'owner-laptop-1' --out $script:SignKey | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'jarvis-sign keygen fallo' }
$script:SignPublicKey = (Get-Content (Join-Path $AgentRoot 'owner.pub') -Raw).Trim()
Write-Step "Clave de firma owner-laptop-1 generada"

# Catalogo pensado para los escenarios: una app real, una rota y una que termina de inmediato
# (para provocar un fallo de verificacion honesto, no simulado).
$policy = @"
version = 1

[risk]
auto_approve_max = "R2"
approval_required_max = "R3"

[signatures]
approver_keys = [{ id = "owner-laptop-1", key = "$($script:SignPublicKey)" }]

[[principals]]
id = "cloud:user:$($script:OwnerId)"
capabilities = ["windows.apps.read", "windows.apps.open", "windows.apps.close", "windows.apps.close.force", "commands.run", "commands.run:wait"]
public_keys = [{ id = "owner-laptop-1", key = "$($script:SignPublicKey)" }]

[[principals]]
id = "local:owner"
capabilities = ["windows.apps.read", "windows.apps.open", "windows.apps.close", "windows.apps.close.force", "commands.run", "commands.run:wait"]

[apps.notepad]
display_name = "Notepad"
launch = { kind = "path", path = '%SystemRoot%\System32\notepad.exe' }
process_names = ["notepad.exe"]

[apps.chrome]
display_name = "Google Chrome"
launch = { kind = "app_paths", name = "chrome.exe" }
process_names = ["chrome.exe"]

# Entrada cuyo ejecutable no existe: sirve para el fallo de ejecucion (WIN-E2E-07).
[apps.broken]
display_name = "Broken App"
launch = { kind = "path", path = '%SystemRoot%\System32\jarvis-no-existe.exe' }
process_names = ["jarvis-no-existe.exe"]

# Proceso que termina de inmediato: la verificacion no lo encuentra vivo (WIN-E2E-08).
[apps.flash]
display_name = "Flash"
launch = { kind = "path", path = '%SystemRoot%\System32\where.exe' }
process_names = ["where.exe"]

[commands.wait]
description = "Espera unos 20 segundos"
executable = '%SystemRoot%\System32\ping.exe'
args = ["-n", "21", "127.0.0.1"]
risk = "R2"
timeout_ms = 30000
expected_exit_codes = [0, 1]

[privacy]
title_mode = "redact_patterns"
"@
$policyPath = Join-Path $AgentRoot 'policy.toml'
Set-Content -Path $policyPath -Value $policy -Encoding utf8

$provision = Invoke-Agent -AgentArgs @(
    'provision', '--device-id', $script:DeviceId, '--host', $BrokerHost, '--port', "$BrokerPort",
    '--plaintext', '--secret-stdin', '--no-policy'
) -StdIn $secret
Save-Evidence 'provision' $provision
if ($provision.ExitCode -ne 0) { throw "provision fallo: $($provision.StdOut) $($provision.StdErr)" }
Write-Step 'Credencial guardada y conexion probada'

# El config.toml se reescribe entero (no se parchea) para habilitar las tools que usan los
# escenarios: es mas robusto que tocar el array `enabled` con expresiones regulares.
$configPath = Join-Path $AgentRoot 'config.toml'
$config = @"
[device]
device_id = "$($script:DeviceId)"

[cloud]
broker_url = "mqtt://${BrokerHost}:${BrokerPort}"

[tools]
enabled = ["is_app_running", "list_running_apps", "get_active_window", "open_app", "close_app", "run_command"]

[logging]
level = "info"
"@
Set-Content -Path $configPath -Value $config -Encoding utf8
Copy-Item $configPath (Join-Path $EvidenceDir 'config.toml')
Copy-Item $policyPath (Join-Path $EvidenceDir 'policy.toml')

Start-Agent
$online = Wait-Device -PresenceState 'Online' -TimeoutSeconds 45
if (-not $online) { Stop-Agent; throw 'El device no llego a Online' }
Save-Evidence 'device-online' $online
Write-Step "Device Online; taskTypes: $($online.supportedTaskTypes -join ', ')"

Write-Host "`n== Escenarios ==" -ForegroundColor Cyan

try {
    # ---- WIN-E2E-01 ----------------------------------------------------------------
    $created = New-Task -Type 'OPEN_APP' -Payload @{ appId = 'notepad' }
    $task = Wait-Task -TaskId $created.id
    Save-Evidence 'e2e-01-open-app' $task
    $ok = $task.status -eq 'Completed' -and $task.result.outcome -eq 'SUCCEEDED' -and
          $task.result.verification.status -eq 'PASSED' -and $task.result.schema -eq 'jarvis.tool-result/1'
    Add-Result 'WIN-E2E-01' 'Tarea valida open_app' (Test-Ok $ok) `
        "status=$($task.status) outcome=$($task.result.outcome) verification=$($task.result.verification.status)"

    # ---- WIN-E2E-02 ----------------------------------------------------------------
    $details = @()
    $allRejected = $true
    foreach ($payload in @(@{ appId = '../../cmd' }, @{ app = 'chrome' }, @{ appId = 'no-existe' })) {
        $invalid = New-Task -Type 'OPEN_APP' -Payload $payload
        if ($invalid.HttpError) {
            # El Cloud tambien puede rechazarlo antes de despachar: sigue cumpliendo "nada se ejecuta".
            $details += "cloud-$($invalid.StatusCode)"
            continue
        }
        $t = Wait-Task -TaskId $invalid.id
        $details += "$($t.status)/$($t.error.code)"
        # El Cloud no tiene estado Rejected: un `task.rejected` del agente queda como Failed con su
        # codigo de error (ver docs/00-overview/cloud-compatibility.md).
        if ($t.status -notin @('Rejected', 'Failed') -or $t.error.code -ne 'INVALID_PAYLOAD') { $allRejected = $false }
    }
    Add-Result 'WIN-E2E-02' 'Tarea invalida' (Test-Ok $allRejected) ($details -join ' ')

    # ---- WIN-E2E-03 ----------------------------------------------------------------
    # "Cloud de test": se publica el mismo `task.dispatch` 100 veces con la credencial del Cloud,
    # directamente contra el broker. El agente debe ejecutarlo una sola vez.
    if (-not (Test-Path $CloudPasswordFile)) {
        Add-Result 'WIN-E2E-03' 'Tarea duplicada' 'SKIP' "no se encontro la credencial del Cloud en $CloudPasswordFile"
    } else {
        $cloudPassword = (Get-Content $CloudPasswordFile -Raw).Trim()
        $dupTask = [guid]::NewGuid().ToString()
        $now = (Get-Date).ToUniversalTime()
        $dispatch = @{
            v        = 1
            id       = [guid]::NewGuid().ToString()
            type     = 'task.dispatch'
            ts       = $now.ToString('o')
            deviceId = $script:DeviceId
            data     = @{
                taskId             = $dupTask
                type               = 'OPEN_APP'
                payload            = @{ appId = 'notepad' }
                attempt            = 1
                createdAt          = $now.ToString('o')
                expiresAt          = $now.AddMinutes(5).ToString('o')
                maxDurationSeconds = 60
                requestedBy        = @{ type = 'User'; id = $script:OwnerId }
                cancelRequested    = $false
            }
        } | ConvertTo-Json -Depth 8 -Compress

        $topic = "jarvis/v1/devices/$($script:DeviceId)/down/tasks"
        # `-i` explicito: el broker rechaza los client id que genera mosquitto_pub por su cuenta.
        $publish = "i=0; while [ `$i -lt 100 ]; do mosquitto_pub -h 127.0.0.1 -u jarvis-cloud " +
                   "-P '$cloudPassword' -i jarvis-cloud-e2e -t '$topic' -f /tmp/jarvis-dup.json -q 1 " +
                   "|| exit 1; i=`$((i+1)); done"
        $published = $true
        try {
            # UTF-8 sin BOM y `docker cp`: por la tuberia de PowerShell el JSON llega con BOM y el
            # agente lo descarta como envelope invalido.
            $payloadFile = Join-Path $EvidenceDir 'e2e-03-dispatch.json'
            [System.IO.File]::WriteAllText($payloadFile, $dispatch, [System.Text.UTF8Encoding]::new($false))
            docker cp $payloadFile "${MosquittoContainer}:/tmp/jarvis-dup.json" 2>&1 | Out-Null
            docker exec $MosquittoContainer sh -c $publish 2>&1 | Out-Null
            if ($LASTEXITCODE -ne 0) { $published = $false }
        } catch { $published = $false }
        Start-Sleep -Seconds 8

        # La evidencia esta en el log del agente: cuantas veces vio el dispatch y cuantas ejecuto.
        # El log de consola lleva codigos de color: se quitan antes de contar.
        $raw = Get-Content (Join-Path $EvidenceDir "agent-console-$($script:AgentRun).log") -Raw
        $log = [regex]::Replace($raw, "\[[0-9;]*m", '')
        $lines = $log -split "`r?`n" | Where-Object { $_ -match [regex]::Escape($dupTask) }
        $seen = $lines.Count
        $finished = ($lines | Where-Object { $_ -match 'task finished' }).Count
        $resent = ($lines | Where-Object { $_ -match 'duplicate dispatch' }).Count
        Save-Evidence 'e2e-03-duplicate' @{
            taskId = $dupTask; logMentions = $seen; executions = $finished; duplicatesAnswered = $resent
        }
        if (-not $published) {
            Add-Result 'WIN-E2E-03' 'Tarea duplicada' 'SKIP' 'el broker rechazo las publicaciones del Cloud de prueba'
        } else {
            $ok = $finished -eq 1 -and $resent -ge 1
            Add-Result 'WIN-E2E-03' 'Tarea duplicada' (Test-Ok $ok) `
                "100 publicaciones -> $finished ejecucion(es), $resent duplicados respondidos con el ultimo estado"
        }
    }

    # ---- WIN-E2E-04 ----------------------------------------------------------------
    $rotated = Invoke-Api -Method POST -Path "/api/v1/devices/$($script:DeviceId)/credentials/rotate"
    # Al rotar, el broker expulsa al cliente; el agente reintenta con la credencial vieja y debe
    # quedarse en AUTH_REJECTED (la presencia en el Cloud tarda mas: depende del heartbeat).
    $authRejected = $false
    $deadline = (Get-Date).AddSeconds(60)
    $statusJson = ''
    while ((Get-Date) -lt $deadline) {
        $statusJson = (Invoke-Agent -AgentArgs @('status', '--json')).StdOut
        if ($statusJson -match 'AUTH_REJECTED' -or $statusJson -match '"authRejected":\s*true') { $authRejected = $true; break }
        Start-Sleep -Seconds 2
    }
    Save-Evidence 'e2e-04-after-rotate' @{ agentStatus = $statusJson }

    Invoke-Agent -AgentArgs @(
        'provision', '--device-id', $script:DeviceId, '--host', $BrokerHost, '--port', "$BrokerPort",
        '--plaintext', '--secret-stdin', '--no-policy'
    ) -StdIn $rotated.secret | Out-Null
    Invoke-Agent -AgentArgs @('reconnect') | Out-Null
    $backOnline = Wait-Device -PresenceState 'Online' -TimeoutSeconds 120
    $ok = $authRejected -and ($null -ne $backOnline)
    Add-Result 'WIN-E2E-04' 'Device revocado' (Test-Ok $ok) `
        "authRejected=$authRejected reonline=$($null -ne $backOnline) (sin reiniciar el agente)"
    Wait-Ready | Out-Null

    # ---- WIN-E2E-05 ----------------------------------------------------------------
    # Se retira la capability de cierre forzado y se pide justo eso.
    Set-Content -Path $policyPath -Value ($policy -replace '"windows\.apps\.close\.force", ', '') -Encoding utf8
    Invoke-Agent -AgentArgs @('policy', 'reload') | Out-Null
    Start-Sleep -Seconds 1
    $denied = New-Task -Type 'CLOSE_APP' -Payload @{ appId = 'notepad'; force = $true }
    $t = Wait-Task -TaskId $denied.id
    Save-Evidence 'e2e-05-not-permitted' $t
    $ok = $t.status -in @('Rejected', 'Failed') -and $t.error.code -eq 'NOT_PERMITTED'
    Add-Result 'WIN-E2E-05' 'Permiso denegado' (Test-Ok $ok) "status=$($t.status) code=$($t.error.code)"
    Set-Content -Path $policyPath -Value $policy -Encoding utf8
    Invoke-Agent -AgentArgs @('policy', 'reload') | Out-Null

    # ---- WIN-E2E-06 ----------------------------------------------------------------
    $unsupported = New-Task -Type 'FORMAT_DISK' -Payload @{}
    if ($unsupported.HttpError) {
        Save-Evidence 'e2e-06-unsupported' $unsupported
        Add-Result 'WIN-E2E-06' 'Tool inexistente' 'PASS' "el Cloud lo rechaza antes de despachar (HTTP $($unsupported.StatusCode))"
    } else {
        $t = Wait-Task -TaskId $unsupported.id
        Save-Evidence 'e2e-06-unsupported' $t
        $ok = $t.status -in @('Rejected', 'Failed') -and $t.error.code -eq 'UNSUPPORTED_TASK_TYPE'
        Add-Result 'WIN-E2E-06' 'Tool inexistente' (Test-Ok $ok) "status=$($t.status) code=$($t.error.code)"
    }

    # ---- WIN-E2E-07 ----------------------------------------------------------------
    $brokenTask = New-Task -Type 'OPEN_APP' -Payload @{ appId = 'broken' }
    $t = Wait-Task -TaskId $brokenTask.id
    Save-Evidence 'e2e-07-failed' $t
    $noStack = -not ($t.error.message -match 'panicked|RUST_BACKTRACE|\\src\\')
    $ok = $t.status -eq 'Failed' -and $t.error.code -eq 'APP_NOT_FOUND' -and $noStack
    Add-Result 'WIN-E2E-07' 'Ejecucion fallida' (Test-Ok $ok) `
        "status=$($t.status) code=$($t.error.code) sinStackTrace=$noStack"

    # ---- WIN-E2E-08 ----------------------------------------------------------------
    # where.exe termina de inmediato: se lanza bien, pero la verificacion no lo encuentra vivo.
    $flash = New-Task -Type 'OPEN_APP' -Payload @{ appId = 'flash' }
    $t = Wait-Task -TaskId $flash.id -TimeoutSeconds 45
    Save-Evidence 'e2e-08-verification' $t
    $ok = $t.status -eq 'Failed' -and $t.error.code -eq 'VERIFICATION_FAILED'
    Add-Result 'WIN-E2E-08' 'Verificacion fallida' (Test-Ok $ok) `
        "status=$($t.status) code=$($t.error.code) verification=$($t.result.verification.status)"

    # ---- WIN-E2E-09 ----------------------------------------------------------------
    $brokerControllable = $false
    try { docker inspect $MosquittoContainer *> $null; $brokerControllable = $LASTEXITCODE -eq 0 } catch {}
    if (-not $brokerControllable) {
        Add-Result 'WIN-E2E-09' 'Desconexion durante la ejecucion' 'SKIP' "no se puede controlar $MosquittoContainer"
    } else {
        $pending = New-Task -Type 'OPEN_APP' -Payload @{ appId = 'notepad' }
        docker stop $MosquittoContainer *> $null
        Start-Sleep -Seconds 3
        docker start $MosquittoContainer *> $null
        $t = Wait-Task -TaskId $pending.id -TimeoutSeconds 150
        Save-Evidence 'e2e-09-disconnect' $t
        $ok = $t.status -eq 'Completed'
        Add-Result 'WIN-E2E-09' 'Desconexion durante la ejecucion' (Test-Ok $ok) `
            "status=$($t.status) (resultado entregado tras reconectar)"
        Wait-Device -PresenceState 'Online' -TimeoutSeconds 120 | Out-Null
    }

    # ---- WIN-E2E-10 ----------------------------------------------------------------
    $beforeRestart = New-Task -Type 'OPEN_APP' -Payload @{ appId = 'notepad' }
    Start-Sleep -Milliseconds 300
    if (-not $script:AgentProcess.HasExited) {
        $script:AgentProcess.Kill()
        $script:AgentProcess.WaitForExit(5000) | Out-Null
    }
    Start-Agent
    Wait-Device -PresenceState 'Online' -TimeoutSeconds 90 | Out-Null
    $t = Wait-Task -TaskId $beforeRestart.id -TimeoutSeconds 120
    Save-Evidence 'e2e-10-restart' $t
    # Tras una muerte brusca hay dos desenlaces honestos: terminada, o reportada como interrumpida.
    $ok = $t.status -eq 'Completed' -or
          ($t.status -eq 'Failed' -and $t.error.code -in @('EXECUTION_INTERRUPTED', 'TOOL_TIMEOUT'))
    Add-Result 'WIN-E2E-10' 'Reinicio con trabajo pendiente' (Test-Ok $ok) "status=$($t.status) code=$($t.error.code)"

    # ---- WIN-E2E-11 ----------------------------------------------------------------
    $long = New-Task -Type 'RUN_COMMAND' -Payload @{ commandId = 'wait' } -MaxDurationSeconds 60
    if ($long.HttpError) {
        Add-Result 'WIN-E2E-11' 'Cancelacion' 'SKIP' "el Cloud no despacha RUN_COMMAND (HTTP $($long.StatusCode))"
    } else {
        Wait-Task -TaskId $long.id -TimeoutSeconds 20 -Until @('Running') | Out-Null
        Invoke-Api -Method POST -Path "/api/v1/tasks/$($long.id)/cancel" -Body @{ reason = 'e2e' } -AllowError | Out-Null
        $t = Wait-Task -TaskId $long.id -TimeoutSeconds 40
        Save-Evidence 'e2e-11-cancel' $t
        $ok = $t.status -eq 'Cancelled' -or ($t.status -eq 'Failed' -and $t.error.code -eq 'EXECUTION_INTERRUPTED')
        Add-Result 'WIN-E2E-11' 'Cancelacion' (Test-Ok $ok) "status=$($t.status) code=$($t.error.code)"
    }

    # ---- WIN-E2E-12 ----------------------------------------------------------------
    $timeout = New-Task -Type 'RUN_COMMAND' -Payload @{ commandId = 'wait' } -MaxDurationSeconds 3
    if ($timeout.HttpError) {
        Add-Result 'WIN-E2E-12' 'Timeout' 'SKIP' "el Cloud no despacha RUN_COMMAND (HTTP $($timeout.StatusCode))"
    } else {
        $t = Wait-Task -TaskId $timeout.id -TimeoutSeconds 60
        Save-Evidence 'e2e-12-timeout' $t
        $ok = $t.status -eq 'Failed' -and $t.error.code -eq 'TOOL_TIMEOUT'
        Add-Result 'WIN-E2E-12' 'Timeout' (Test-Ok $ok) "status=$($t.status) code=$($t.error.code)"
    }

    # ---- WIN-E2E-13 ----------------------------------------------------------------
    $r3 = New-Task -Type 'CLOSE_APP' -Payload @{ appId = 'notepad'; force = $true } -MaxDurationSeconds 120
    $waiting = Wait-Task -TaskId $r3.id -TimeoutSeconds 25 `
        -Until @('WaitingForApproval', 'Completed', 'Failed', 'Rejected')
    $pendingList = Invoke-Agent -AgentArgs @('approvals', 'list')
    $granted = Invoke-Agent -AgentArgs @('approvals', 'approve', $r3.id)
    $t = Wait-Task -TaskId $r3.id -TimeoutSeconds 90
    Save-Evidence 'e2e-13-approval' @{ waiting = $waiting; pending = $pendingList.StdOut; grant = $granted.StdOut; final = $t }
    $ok = $granted.ExitCode -eq 0 -and $t.status -eq 'Completed'
    Add-Result 'WIN-E2E-13' 'Aprobacion local R3' (Test-Ok $ok) "espera=$($waiting.status) final=$($t.status)"

    # ---- WIN-E2E-14 ----------------------------------------------------------------
    $alias = New-Task -Type 'OPEN_CHROME' -Payload @{}
    if ($alias.HttpError) {
        Add-Result 'WIN-E2E-14' 'Alias OPEN_CHROME' 'SKIP' "el Cloud no acepta el alias (HTTP $($alias.StatusCode))"
    } else {
        $t = Wait-Task -TaskId $alias.id -TimeoutSeconds 90
        Save-Evidence 'e2e-14-alias' $t
        # Si Chrome no esta instalado, APP_NOT_FOUND tambien demuestra que el alias se resolvio.
        $ok = $t.status -eq 'Completed' -or ($t.status -eq 'Failed' -and $t.error.code -eq 'APP_NOT_FOUND')
        Add-Result 'WIN-E2E-14' 'Alias OPEN_CHROME' (Test-Ok $ok) "status=$($t.status) code=$($t.error.code)"
    }

    # ---- WIN-E2E-15 ----------------------------------------------------------------
    # Tarea firmada de extremo a extremo: firma el cliente del owner, el Cloud la transporta y el
    # agente la verifica contra su policy.toml. Con require_above = "R2" la firma pasa de ser
    # opcional a obligatoria para R3, que es el escenario que de verdad importa.
    Set-Content -Path $policyPath -Encoding utf8 -Value ($policy -replace `
        '\[signatures\]', "[signatures]`nrequire_above = `"R2`"")
    Invoke-Agent -AgentArgs @('policy', 'reload') | Out-Null
    Start-Sleep -Milliseconds 500

    $signed = New-SignedTask -Type 'OPEN_APP' -Payload @{ appId = 'notepad' }
    $created = Invoke-Api -Method POST -Path '/api/v1/tasks' -Body $signed.Body -AllowError
    $signedResult = $null
    if (-not $created.HttpError) { $signedResult = Wait-Task -TaskId $created.id -TimeoutSeconds 90 }

    # Y la misma tarea con un byte cambiado en la firma: el Cloud no la verifica (no tiene la clave),
    # asi que tiene que ser el agente quien la rechace.
    $tampered = New-SignedTask -Type 'OPEN_APP' -Payload @{ appId = 'notepad' }
    $value = $tampered.Body.signature.value
    $first = if ($value[0] -eq 'A') { 'B' } else { 'A' }
    $tampered.Body.signature.value = "$first$($value.Substring(1))"
    $forged = Invoke-Api -Method POST -Path '/api/v1/tasks' -Body $tampered.Body -AllowError
    $forgedResult = $null
    if (-not $forged.HttpError) { $forgedResult = Wait-Task -TaskId $forged.id -TimeoutSeconds 60 }

    Save-Evidence 'e2e-15-signed-task' @{
        accepted = $signedResult; tamperedAccepted = $forged.HttpError -eq $null; tampered = $forgedResult
    }
    $ok = $signedResult -and $signedResult.status -eq 'Completed' `
        -and $forgedResult -and $forgedResult.status -eq 'Failed' `
        -and $forgedResult.error.code -eq 'NOT_PERMITTED' `
        -and $forgedResult.error.message -match 'SIGNATURE_INVALID'
    Add-Result 'WIN-E2E-15' 'Tarea firmada end-to-end' (Test-Ok $ok) `
        "firmada=$($signedResult.status) alterada=$($forgedResult.status)/$($forgedResult.error.code) ($($forgedResult.error.message))"

    # ---- WIN-E2E-16 ----------------------------------------------------------------
    # Aprobacion remota: tarea R3 firmada -> el agente pide aprobacion -> el owner firma la decision
    # -> el Cloud la transmite -> el agente verifica firma, nonce y frescura, y la aplica.
    $r3Payload = @{ appId = 'notepad'; force = $true }
    $r3Signed = New-SignedTask -Type 'CLOSE_APP' -Payload $r3Payload
    $r3Created = Invoke-Api -Method POST -Path '/api/v1/tasks' -Body $r3Signed.Body -AllowError
    if ($r3Created.HttpError) {
        Add-Result 'WIN-E2E-16' 'Aprobacion remota firmada' 'SKIP' "el Cloud rechazo la tarea firmada (HTTP $($r3Created.StatusCode))"
    } else {
        $waiting = Wait-Task -TaskId $r3Created.id -TimeoutSeconds 30 -Until @('WaitingForApproval', 'Completed', 'Failed', 'Rejected')
        $decision = Invoke-Sign @(
            'decision', '--key', $script:SignKey, '--task-id', $r3Created.id,
            '--payload-file', $r3Signed.PayloadFile, '--granted'
        )
        $sent = Invoke-Api -Method POST -Path "/api/v1/tasks/$($r3Created.id)/approval" -Body $decision -AllowError
        $final = Wait-Task -TaskId $r3Created.id -TimeoutSeconds 90
        Save-Evidence 'e2e-16-remote-approval' @{ waiting = $waiting; relayed = -not $sent.HttpError; final = $final }
        $ok = $waiting.status -eq 'WaitingForApproval' -and (-not $sent.HttpError) -and $final.status -eq 'Completed'
        Add-Result 'WIN-E2E-16' 'Aprobacion remota firmada' (Test-Ok $ok) `
            "espera=$($waiting.status) transmitida=$(-not $sent.HttpError) final=$($final.status)"
    }

    # Se restaura la politica sin exigencia de firma para no afectar a nada posterior.
    Set-Content -Path $policyPath -Value $policy -Encoding utf8
    Invoke-Agent -AgentArgs @('policy', 'reload') | Out-Null
}
finally {
    Write-Host "`n== Cierre ==" -ForegroundColor Cyan
    $finalStatus = Invoke-Agent -AgentArgs @('status', '--json')
    Set-Content -Path (Join-Path $EvidenceDir 'agent-status.json') -Value $finalStatus.StdOut -Encoding utf8
    Stop-Agent
    if (Test-Path (Join-Path $AgentRoot 'logs')) {
        Copy-Item (Join-Path $AgentRoot 'logs') (Join-Path $EvidenceDir 'logs') -Recurse -ErrorAction SilentlyContinue
    }
    if (-not $KeepDevice -and $script:DeviceId) {
        Invoke-Api -Method POST -Path "/api/v1/devices/$($script:DeviceId)/revoke" `
            -Body @{ reason = 'e2e finalizado' } -AllowError | Out-Null
    }

    Save-Evidence 'summary' ([pscustomobject]@{
        RunAt = (Get-Date).ToString('o'); ApiBase = $ApiBase; DeviceId = $script:DeviceId
        AgentExe = $AgentExe; Results = $script:Results
    })
    $script:Results | Format-Table -AutoSize | Out-String | Write-Host
    $pass = ($script:Results | Where-Object Status -eq 'PASS').Count
    $fail = ($script:Results | Where-Object Status -eq 'FAIL').Count
    $skip = ($script:Results | Where-Object Status -eq 'SKIP').Count
    Write-Host "PASS=$pass FAIL=$fail SKIP=$skip   evidencia: $EvidenceDir" -ForegroundColor Cyan
    if ($fail -gt 0) { exit 1 }
}
