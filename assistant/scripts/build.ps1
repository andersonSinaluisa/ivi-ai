# Genera el instalador de Windows (NSIS) en apps\desktop\src-tauri\target\release\bundle\nsis
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
npm run tauri:build -w apps/desktop
