[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)]
  [string]$StagedBinary,
  [Parameter(Mandatory = $true)]
  [string]$AppBinary,
  [int]$WaitForPid = 0,
  [int]$Attempts = 120,
  [int]$RetryDelayMs = 500,
  [string]$StatusFile
)

$ErrorActionPreference = "Stop"
$StagedBinary = [System.IO.Path]::GetFullPath($StagedBinary)
$AppBinary = [System.IO.Path]::GetFullPath($AppBinary)
if ($StatusFile) {
  $StatusFile = [System.IO.Path]::GetFullPath($StatusFile)
}
$SelfSignature = Get-AuthenticodeSignature -LiteralPath $PSCommandPath
if ($SelfSignature.Status -ne [System.Management.Automation.SignatureStatus]::Valid) {
  throw "The Petri installer helper must have a valid Authenticode signature."
}
$ExpectedPublisherThumbprint = $SelfSignature.SignerCertificate.Thumbprint
if ([string]::IsNullOrWhiteSpace($ExpectedPublisherThumbprint)) {
  throw "The Petri installer helper signature does not identify a publisher certificate."
}

function Write-UpdateStatus([string]$Status, [string]$Message, [int]$Attempt) {
  if (-not $StatusFile) {
    return
  }
  $StatusDirectory = Split-Path -Parent $StatusFile
  New-Item -ItemType Directory -Force -Path $StatusDirectory | Out-Null
  $TemporaryStatus = "$StatusFile.tmp.$PID"
  $Payload = [ordered]@{
    status = $Status
    message = $Message
    appBinary = $AppBinary
    updaterPid = $PID
    attempt = $Attempt
    updatedAt = [DateTimeOffset]::UtcNow.ToString("o")
  } | ConvertTo-Json -Compress
  [System.IO.File]::WriteAllText($TemporaryStatus, $Payload, (New-Object System.Text.UTF8Encoding($false)))
  Remove-Item -LiteralPath $StatusFile -Force -ErrorAction SilentlyContinue
  Move-Item -LiteralPath $TemporaryStatus -Destination $StatusFile -Force
}

if (-not (Test-Path -LiteralPath $StagedBinary -PathType Leaf)) {
  Write-UpdateStatus "failed" "The staged Petri binary does not exist." 0
  throw "Missing staged Petri binary: $StagedBinary"
}
$StagedSignature = Get-AuthenticodeSignature -LiteralPath $StagedBinary
if ($StagedSignature.Status -ne [System.Management.Automation.SignatureStatus]::Valid) {
  Write-UpdateStatus "failed" "The staged Petri binary has an invalid Authenticode signature." 0
  throw "The staged Petri binary must have a valid Authenticode signature."
}
if ($StagedSignature.SignerCertificate.Thumbprint -ine $ExpectedPublisherThumbprint) {
  Write-UpdateStatus "failed" "The staged Petri binary is signed by a different publisher." 0
  throw "The staged Petri binary must be signed by the Petri installer helper publisher."
}
$ExpectedBinaryHash = (Get-FileHash -LiteralPath $StagedBinary -Algorithm SHA256).Hash

if ($WaitForPid -gt 0) {
  Write-UpdateStatus "waiting" "Waiting for Petri process $WaitForPid to exit." 0
  Wait-Process -Id $WaitForPid -ErrorAction SilentlyContinue
}

$LastFailure = $null
for ($Attempt = 1; $Attempt -le [Math]::Max(1, $Attempts); $Attempt++) {
  $BackupBinary = "$AppBinary.old.$PID.$Attempt"
  $MovedExistingApp = $false
  try {
    if (Test-Path -LiteralPath $BackupBinary) {
      Remove-Item -LiteralPath $BackupBinary -Force
    }
    if (Test-Path -LiteralPath $AppBinary) {
      [System.IO.File]::Move($AppBinary, $BackupBinary)
      $MovedExistingApp = $true
    }

    try {
      $StagedSignature = Get-AuthenticodeSignature -LiteralPath $StagedBinary
      if ($StagedSignature.Status -ne [System.Management.Automation.SignatureStatus]::Valid) {
        throw "The staged Petri binary signature became invalid before replacement."
      }
      if ($StagedSignature.SignerCertificate.Thumbprint -ine $ExpectedPublisherThumbprint) {
        throw "The staged Petri binary publisher changed before replacement."
      }
      if ((Get-FileHash -LiteralPath $StagedBinary -Algorithm SHA256).Hash -ine $ExpectedBinaryHash) {
        throw "The staged Petri binary changed before replacement."
      }
      [System.IO.File]::Move($StagedBinary, $AppBinary)
      $InstalledSignature = Get-AuthenticodeSignature -LiteralPath $AppBinary
      if ($InstalledSignature.Status -ne [System.Management.Automation.SignatureStatus]::Valid -or
          $InstalledSignature.SignerCertificate.Thumbprint -ine $ExpectedPublisherThumbprint) {
        throw "The installed Petri binary did not retain the trusted publisher signature."
      }
      if ((Get-FileHash -LiteralPath $AppBinary -Algorithm SHA256).Hash -ine $ExpectedBinaryHash) {
        throw "The installed Petri binary does not match the verified staged binary."
      }
    } catch {
      if (Test-Path -LiteralPath $AppBinary) {
        Remove-Item -LiteralPath $AppBinary -Force
      }
      if ($MovedExistingApp -and (Test-Path -LiteralPath $BackupBinary)) {
        [System.IO.File]::Move($BackupBinary, $AppBinary)
      }
      throw
    }

    Remove-Item -LiteralPath $BackupBinary -Force -ErrorAction SilentlyContinue
    Get-ChildItem -LiteralPath (Split-Path -Parent $AppBinary) -Filter "$(Split-Path -Leaf $AppBinary).old.*" -File -ErrorAction SilentlyContinue |
      Remove-Item -Force -ErrorAction SilentlyContinue
    Write-UpdateStatus "installed" "Petri was updated successfully." $Attempt
    exit 0
  } catch {
    $LastFailure = $_.Exception.Message
    if ($Attempt -lt [Math]::Max(1, $Attempts)) {
      Start-Sleep -Milliseconds ([Math]::Max(50, $RetryDelayMs))
    }
  }
}

Write-UpdateStatus "failed" "Petri could not be replaced: $LastFailure" ([Math]::Max(1, $Attempts))
throw "Could not replace $AppBinary after $Attempts attempts. $LastFailure"
