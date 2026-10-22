# Busca un Python compatible con el servicio de voz (>= 3.11 y < 3.13).
# Prueba el lanzador "py" (instalador de python.org) y luego python3.11 / python3.12 / python en el PATH
# (winget, Microsoft Store, pyenv-win). Ignora el alias vacio de la Microsoft Store.
function Find-JarvisPython {
    $candidates = @(
        @{ Exe = 'py'; Args = @('-3.11') },
        @{ Exe = 'py'; Args = @('-3.12') },
        @{ Exe = 'python3.11'; Args = @() },
        @{ Exe = 'python3.12'; Args = @() },
        @{ Exe = 'python'; Args = @() },
        @{ Exe = 'python3'; Args = @() }
    )
    foreach ($c in $candidates) {
        if (-not (Get-Command $c.Exe -ErrorAction SilentlyContinue)) { continue }
        try {
            $cmdArgs = @($c.Args) + @('--version')
            $out = (& $c.Exe @cmdArgs 2>&1 | Out-String).Trim()
        } catch { continue }
        if ($LASTEXITCODE -eq 0 -and $out -match '^Python 3\.(11|12)\.') {
            return [pscustomobject]@{ Exe = $c.Exe; Args = $c.Args; Version = $out; Display = (@($c.Exe) + $c.Args) -join ' ' }
        }
    }
    return $null
}
