[CmdletBinding()]
param([string]$InstallRoot = (Join-Path $env:LOCALAPPDATA 'Programs\PetriPreview'), [switch]$NoShortcut)
$ErrorActionPreference = 'Stop'
$Package = $PSScriptRoot
if (-not (Test-Path -LiteralPath (Join-Path $Package 'petri.exe'))) {
  $Stage = Join-Path ([IO.Path]::GetTempPath()) ('petri-download-' + [Guid]::NewGuid().ToString('N'))
  New-Item -ItemType Directory -Path $Stage | Out-Null
  $Base = 'https://github.com/amoeba-farm/petri/releases/latest/download/Petri-windows-x64.zip'
  $Archive = Join-Path $Stage 'Petri.zip'
  Invoke-WebRequest -UseBasicParsing -Uri $Base -OutFile $Archive
  $Expected = ((Invoke-WebRequest -UseBasicParsing -Uri ($Base + '.sha256')).Content.Trim() -split '\s+')[0]
  if ($Expected -notmatch '^[a-fA-F0-9]{64}$' -or (Get-FileHash -LiteralPath $Archive -Algorithm SHA256).Hash -ne $Expected) { throw 'Release checksum mismatch' }
  Expand-Archive -LiteralPath $Archive -DestinationPath $Stage
  $Package = Join-Path $Stage 'Petri-windows-x64'
}
$Package = (Resolve-Path -LiteralPath $Package).Path
foreach ($Line in Get-Content -LiteralPath (Join-Path $Package 'SHA256SUMS')) {
  if ($Line -notmatch '^([a-f0-9]{64})  (.+)$') { throw 'Invalid package checksum manifest' }
  $Hash = $Matches[1]; $Relative = $Matches[2]
  $File = [IO.Path]::GetFullPath((Join-Path $Package $Relative))
  if (-not $File.StartsWith($Package + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Unsafe package path' }
  if ((Get-FileHash -LiteralPath $File -Algorithm SHA256).Hash -ne $Hash) { throw "Package checksum mismatch: $Relative" }
}
$InstallRoot = [IO.Path]::GetFullPath($InstallRoot)
New-Item -ItemType Directory -Force -Path $InstallRoot | Out-Null
foreach ($Name in @('petri.exe','Petri.cmd','Petri.ico','LICENSE','THIRD_PARTY_NOTICES.md','THIRD_PARTY_LICENSES.md','README.txt')) {
  Copy-Item -LiteralPath (Join-Path $Package $Name) -Destination (Join-Path $InstallRoot $Name) -Force
}
if (Test-Path -LiteralPath (Join-Path $Package 'petri-update.json')) {
  Copy-Item -LiteralPath (Join-Path $Package 'petri-update.json') -Destination (Join-Path $InstallRoot 'petri-update.json') -Force
}
$UserPath = [string][Environment]::GetEnvironmentVariable('Path','User')
if (($UserPath -split ';') -notcontains $InstallRoot) { [Environment]::SetEnvironmentVariable('Path', (($UserPath.TrimEnd(';') + ';' + $InstallRoot).TrimStart(';')), 'User') }
if (-not $NoShortcut) {
  $Shell = New-Object -ComObject WScript.Shell
  $Shortcut = $Shell.CreateShortcut((Join-Path ([Environment]::GetFolderPath('Programs')) 'Petri.lnk'))
  $Shortcut.TargetPath = Join-Path $InstallRoot 'petri.exe'
  $Shortcut.Arguments = 'tui'
  $Shortcut.WorkingDirectory = $InstallRoot
  $Shortcut.IconLocation = Join-Path $InstallRoot 'Petri.ico'
  $Shortcut.Save()
}
& (Join-Path $InstallRoot 'petri.exe') --version
if ($LASTEXITCODE -ne 0) { throw 'Installed binary did not start' }
Write-Host 'Petri installed. Open Petri from Start, or restart your terminal and run petri. This is an unsigned Devnet preview.'
