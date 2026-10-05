param([string[]]$Args = @())

$ErrorActionPreference = 'Stop'

function Find-Blender {
    $onPath = Get-Command blender.exe -ErrorAction SilentlyContinue
    if ($onPath) { return $onPath.Source }

    $registry = Get-ItemProperty `
        'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*', `
        'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*' `
        -ErrorAction SilentlyContinue |
        Where-Object { $_.DisplayName -like '*Blender*' -and $_.InstallLocation }
    foreach ($entry in $registry) {
        $candidate = Join-Path $entry.InstallLocation 'blender.exe'
        if (Test-Path $candidate) { return $candidate }
    }

    $roots = @(
        "$env:ProgramFiles\Blender Foundation",
        "${env:ProgramFiles(x86)}\Blender Foundation",
        "$env:LOCALAPPDATA\Programs\Blender Foundation"
    )
    foreach ($root in $roots) {
        if (-not (Test-Path $root)) { continue }
        $found = Get-ChildItem $root -Filter blender.exe -Recurse -ErrorAction SilentlyContinue |
            Sort-Object FullName -Descending | Select-Object -First 1
        if ($found) { return $found.FullName }
    }
    return $null
}

$blender = Find-Blender
if (-not $blender) {
    Write-Error "Could not find Blender. Install it from blender.org, or put blender.exe on PATH."
}

Write-Host "Using $blender"
$root = Split-Path -Parent $PSScriptRoot
$script = Join-Path $PSScriptRoot 'export_hand.py'
& $blender --background --python $script -- @Args
if ($LASTEXITCODE -ne 0) { Write-Error "Blender exited with code $LASTEXITCODE" }
