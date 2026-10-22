# Equivalente Windows de generate-secrets.sh para desarrollo local (.local-secrets en la raíz de api/).
param([string]$Directory = (Join-Path $PSScriptRoot "..\..\.local-secrets"))
New-Item -ItemType Directory -Force $Directory | Out-Null
$names = @{ postgres_password = 32; jwt_signing_key = 48; device_secret_pepper = 48; mqtt_cloud_password = 32; mosquitto_dynsec_admin_password = 32 }
foreach ($name in $names.Keys) {
  $path = Join-Path $Directory $name
  if (-not (Test-Path $path)) {
    $bytes = New-Object byte[] $names[$name]
    [System.Security.Cryptography.RandomNumberGenerator]::Fill($bytes)
    $value = [Convert]::ToBase64String($bytes).TrimEnd('=').Replace('+', '-').Replace('/', '_')
    [System.IO.File]::WriteAllText($path, $value)
    "created $name"
  }
}
"secrets ready in $Directory"
