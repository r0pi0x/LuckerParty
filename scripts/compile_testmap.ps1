# Windows: build tools/testmap/mashup_logic_test.vmf with Valve's map
# compilers and put the .bsp in mashup's map cache, so `map
# cs_source:mashup_logic_test` loads it (and CS:S can too, see below).
#
# Usage (from the repo root):
#   .\scripts\compile_testmap.ps1 -Game "C:\Program Files (x86)\Steam\steamapps\common\Counter-Strike Source"
#   [-Bin <folder with vbsp.exe, vvis.exe, vrad.exe>] [-Fast] [-CopyToGame]
#
# CS:S ships the compilers in its bin\ folder on some installs; otherwise
# use "Source SDK Base 2013 Multiplayer"\bin (free on Steam) with -Bin.
# -Fast skips vvis/vrad detail (quick iteration). -CopyToGame also copies
# the map into cstrike\maps so the real game (and the probe server) can
# load it for measurements.
param(
    [Parameter(Mandatory = $true)][string]$Game,
    [string]$Bin = "",
    [switch]$Fast,
    [switch]$CopyToGame
)
$ErrorActionPreference = "Stop"

$vmf = Join-Path $PSScriptRoot "..\tools\testmap\mashup_logic_test.vmf" | Resolve-Path
$gamedir = Join-Path $Game "cstrike"
if (-not $Bin) { $Bin = Join-Path $Game "bin" }
foreach ($tool in "vbsp.exe", "vvis.exe", "vrad.exe") {
    if (-not (Test-Path (Join-Path $Bin $tool))) {
        throw "$tool not found in $Bin: pass -Bin with the Source SDK Base 2013 Multiplayer bin folder"
    }
}

# Regenerate the .vmf from the generator first, so it matches the code.
cargo run --bin testmap

$work = Join-Path $env:TEMP "mashup_testmap"
New-Item -ItemType Directory -Force -Path $work | Out-Null
Copy-Item $vmf (Join-Path $work "mashup_logic_test.vmf") -Force
$map = Join-Path $work "mashup_logic_test"

& (Join-Path $Bin "vbsp.exe") -game $gamedir $map
if ($LASTEXITCODE -ne 0) { throw "vbsp failed" }
if ($Fast) {
    & (Join-Path $Bin "vvis.exe") -fast -game $gamedir $map
    & (Join-Path $Bin "vrad.exe") -fast -ldr -game $gamedir $map
} else {
    & (Join-Path $Bin "vvis.exe") -game $gamedir $map
    & (Join-Path $Bin "vrad.exe") -ldr -game $gamedir $map
}
if ($LASTEXITCODE -ne 0) { throw "vrad failed" }

$cache = Join-Path $env:LOCALAPPDATA "mashup\content\cs_source\maps"
New-Item -ItemType Directory -Force -Path $cache | Out-Null
Copy-Item "$map.bsp" $cache -Force
Write-Host "mashup: map cs_source:mashup_logic_test ($cache)"
if ($CopyToGame) {
    Copy-Item "$map.bsp" (Join-Path $gamedir "maps") -Force
    Write-Host "CS:S: map mashup_logic_test"
}
