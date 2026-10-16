[CmdletBinding()]
param(
    [ValidateSet('smoke', 'startup', 'help-keyboard-preview', 'help-mouse-move', 'guide-off', 'help-resize')]
    [string]$Scenario = 'smoke',
    [string]$PetriBinary = (Join-Path $PSScriptRoot '..\..\target\release\petri.exe'),
    [int]$TimeoutMs = 15000,
    [string]$Artifacts = '',
    [switch]$ShowScreen
)

$manifest = Join-Path $PSScriptRoot 'Cargo.toml'
$cargoArgs = @(
    'run',
    '--manifest-path', $manifest,
    '--locked',
    '--',
    $Scenario,
    '--binary', $PetriBinary,
    '--timeout-ms', $TimeoutMs
)

if ($Artifacts) {
    $cargoArgs += @('--artifacts', $Artifacts)
}
if ($ShowScreen) {
    $cargoArgs += '--show-screen'
}

& cargo @cargoArgs
exit $LASTEXITCODE

