[CmdletBinding()]
param(
  [switch]$SkipBuild,
  [switch]$BuildOnly,
  [string]$OutputRoot
)

$ErrorActionPreference = "Stop"
if ($BuildOnly -and $SkipBuild) {
  throw "-BuildOnly and -SkipBuild cannot be used together."
}

$RepoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$ReleaseBinary = Join-Path $RepoRoot "target\release\petri.exe"
$ApplicationIcon = Join-Path $RepoRoot "assets\Petri.ico"
$LicensePath = Join-Path $RepoRoot "LICENSE"
$ThirdPartyNoticesPath = Join-Path $RepoRoot "THIRD_PARTY_NOTICES.md"
$ThirdPartyLicensesPath = Join-Path $RepoRoot "THIRD_PARTY_LICENSES.md"
$InstallerSource = Join-Path $PSScriptRoot "install-petri.ps1"
$UpdaterSource = Join-Path $PSScriptRoot "finish-petri-windows-update.ps1"
if (-not $OutputRoot) {
  $OutputRoot = Join-Path $RepoRoot "dist"
}
$OutputRoot = [System.IO.Path]::GetFullPath($OutputRoot)
$Architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLowerInvariant()
$PackageName = "Petri-windows-$Architecture"
$PackageRoot = Join-Path $OutputRoot $PackageName
$ZipPath = Join-Path $OutputRoot "$PackageName.zip"

if (-not $SkipBuild) {
  & node (Join-Path $PSScriptRoot "build-sdk-runtime.mjs")
  if ($LASTEXITCODE -ne 0) { throw "Pinned SDK runtime build failed" }
  $env:PETRI_REQUIRE_SDK_RUNTIME = "1"
  $RustFlagSeparator = [char]0x1f
  $RemapFlags = @("--remap-path-prefix=$RepoRoot=.", "--remap-path-prefix=$HOME=<home>")
  foreach ($RemapFlag in $RemapFlags) {
    if ($env:CARGO_ENCODED_RUSTFLAGS) {
      $env:CARGO_ENCODED_RUSTFLAGS += "$RustFlagSeparator$RemapFlag"
    } else {
      $env:CARGO_ENCODED_RUSTFLAGS = $RemapFlag
    }
  }
  & cargo build --release --locked --bin petri
  if ($LASTEXITCODE -ne 0) {
    throw "cargo build --release failed with exit code $LASTEXITCODE"
  }
}
& node (Join-Path $PSScriptRoot "build-sdk-runtime.mjs") --verify
if ($LASTEXITCODE -ne 0) { throw "SDK runtime metadata is not current" }
if (-not (Test-Path -LiteralPath $ReleaseBinary)) {
  throw "Missing release binary: $ReleaseBinary"
}
if ($BuildOnly) {
  Write-Host "Built the unsigned Petri release binary:"
  Write-Host "  $ReleaseBinary"
  Write-Host "Sign this file plus the installer and helper scripts with Authenticode, then rerun with -SkipBuild:"
  Write-Host "  $InstallerSource"
  Write-Host "  $UpdaterSource"
  return
}
if (-not (Test-Path -LiteralPath $ApplicationIcon)) {
  throw "Missing application icon: $ApplicationIcon"
}
if (-not (Test-Path -LiteralPath $LicensePath -PathType Leaf)) {
  throw "Missing Apache-2.0 license: $LicensePath"
}
if (-not (Test-Path -LiteralPath $ThirdPartyNoticesPath -PathType Leaf)) {
  throw "Missing third-party notices: $ThirdPartyNoticesPath"
}
if (-not (Test-Path -LiteralPath $ThirdPartyLicensesPath -PathType Leaf)) {
  throw "Missing third-party license corpus: $ThirdPartyLicensesPath"
}
$ReleaseScriptSignatures = @{}
foreach ($ReleaseScript in @($InstallerSource, $UpdaterSource)) {
  if (-not (Test-Path -LiteralPath $ReleaseScript -PathType Leaf)) {
    throw "Missing release script: $ReleaseScript"
  }
  $ScriptSignature = Get-AuthenticodeSignature -LiteralPath $ReleaseScript
  if ($ScriptSignature.Status -ne [System.Management.Automation.SignatureStatus]::Valid) {
    throw "Release script must have a valid Authenticode signature before packaging: $ReleaseScript"
  }
  $ReleaseScriptSignatures[$ReleaseScript] = $ScriptSignature
}
$Signature = Get-AuthenticodeSignature -LiteralPath $ReleaseBinary
if ($Signature.Status -ne [System.Management.Automation.SignatureStatus]::Valid) {
  throw "Petri release binary must have a valid Authenticode signature before packaging."
}
$PublisherThumbprint = $Signature.SignerCertificate.Thumbprint
if ([string]::IsNullOrWhiteSpace($PublisherThumbprint)) {
  throw "The signed Petri release binary does not identify a publisher certificate."
}
foreach ($ReleaseScript in @($InstallerSource, $UpdaterSource)) {
  if ($ReleaseScriptSignatures[$ReleaseScript].SignerCertificate.Thumbprint -ine $PublisherThumbprint) {
    throw "The Petri binary, installer, and updater helper must use the same publisher certificate."
  }
}
$SourceBinaryHash = (Get-FileHash -LiteralPath $ReleaseBinary -Algorithm SHA256).Hash

if (Test-Path -LiteralPath $PackageRoot) {
  Remove-Item -LiteralPath $PackageRoot -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $PackageRoot | Out-Null
Copy-Item -LiteralPath $ReleaseBinary -Destination (Join-Path $PackageRoot "petri.exe")
Copy-Item -LiteralPath $ApplicationIcon -Destination (Join-Path $PackageRoot "Petri.ico")
Copy-Item -LiteralPath $LicensePath -Destination (Join-Path $PackageRoot "LICENSE")
Copy-Item -LiteralPath $ThirdPartyNoticesPath -Destination (Join-Path $PackageRoot "THIRD_PARTY_NOTICES.md")
Copy-Item -LiteralPath $ThirdPartyLicensesPath -Destination (Join-Path $PackageRoot "THIRD_PARTY_LICENSES.md")
Copy-Item -LiteralPath $InstallerSource -Destination (Join-Path $PackageRoot "install-petri.ps1")
Copy-Item -LiteralPath $UpdaterSource -Destination (Join-Path $PackageRoot "finish-petri-windows-update.ps1")
$PackagedBinary = Join-Path $PackageRoot "petri.exe"
$PackagedSignature = Get-AuthenticodeSignature -LiteralPath $PackagedBinary
if ($PackagedSignature.Status -ne [System.Management.Automation.SignatureStatus]::Valid) {
  throw "The packaged Petri binary does not retain a valid Authenticode signature."
}
$PackagedBinaryHash = (Get-FileHash -LiteralPath $PackagedBinary -Algorithm SHA256).Hash
if ($PackagedBinaryHash -ne $SourceBinaryHash) {
  throw "The packaged Petri binary does not match the signed release binary."
}
foreach ($PackagedScriptName in @("install-petri.ps1", "finish-petri-windows-update.ps1")) {
  $PackagedScript = Join-Path $PackageRoot $PackagedScriptName
  $PackagedScriptSignature = Get-AuthenticodeSignature -LiteralPath $PackagedScript
  if ($PackagedScriptSignature.Status -ne [System.Management.Automation.SignatureStatus]::Valid) {
    throw "Packaged release script does not retain a valid Authenticode signature: $PackagedScriptName"
  }
  if ($PackagedScriptSignature.SignerCertificate.Thumbprint -ine $PublisherThumbprint) {
    throw "Packaged release script publisher does not match the Petri binary: $PackagedScriptName"
  }
}

$Readme = @"
Petri for Windows
=================

Double-click install-petri.ps1 with PowerShell, or run:

  powershell -ExecutionPolicy Bypass -File .\install-petri.ps1

The installer adds Petri to the Start Menu and Desktop and installs the
petri command in %USERPROFILE%\.cargo\bin. The package also contains the
project license, third-party inventory, and full third-party license corpus.
"@
Set-Content -LiteralPath (Join-Path $PackageRoot "README.txt") -Value $Readme -Encoding UTF8

$PackageHashes = Get-ChildItem -LiteralPath $PackageRoot -File |
  Where-Object { $_.Name -ne "SHA256SUMS.txt" } |
  Sort-Object Name |
  ForEach-Object {
    $Hash = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    "$Hash  $($_.Name)"
  }
Set-Content -LiteralPath (Join-Path $PackageRoot "SHA256SUMS.txt") -Value $PackageHashes -Encoding ASCII

if (Test-Path -LiteralPath $ZipPath) {
  Remove-Item -LiteralPath $ZipPath -Force
}
Compress-Archive -LiteralPath $PackageRoot -DestinationPath $ZipPath
$ZipHash = (Get-FileHash -LiteralPath $ZipPath -Algorithm SHA256).Hash.ToLowerInvariant()
Set-Content -LiteralPath "$ZipPath.sha256" -Value "$ZipHash  $([System.IO.Path]::GetFileName($ZipPath))" -Encoding ASCII

Write-Host "Created Petri Windows app package:"
Write-Host "  Folder: $PackageRoot"
Write-Host "  Zip:    $ZipPath"
Write-Host "  SHA256: $ZipPath.sha256"
