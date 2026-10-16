[CmdletBinding()]
param(
  [switch]$SkipBuild,
  [switch]$NoDesktopShortcut,
  [string]$InstallRoot,
  [int]$WaitForPid = 0
)

$ErrorActionPreference = "Stop"

$RepoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$RepoManifest = Join-Path $RepoRoot "Cargo.toml"
$CommandName = "petri"
$ShimFileName = "petri.cmd"
$ShortcutName = "Petri"
$ExpectedPublisherThumbprint = $null

if ($SkipBuild) {
  # Source-checkout update mode is an explicit local trust boundary. It never
  # builds and accepts only the fixed release output and assets in this repo.
  $CheckoutInstaller = [System.IO.Path]::GetFullPath((Join-Path $RepoRoot "scripts\install-petri.ps1"))
  if ([System.IO.Path]::GetFullPath($PSCommandPath) -ine $CheckoutInstaller) {
    throw "-SkipBuild must be run from the source checkout's scripts\install-petri.ps1."
  }
  if (-not (Test-Path -LiteralPath $RepoManifest -PathType Leaf)) {
    throw "-SkipBuild is available only from a Petri source checkout; Cargo.toml was not found at $RepoManifest"
  }
  $SourceBinary = Join-Path $RepoRoot "target\release\petri.exe"
  $SourceIcon = Join-Path $RepoRoot "assets\Petri.ico"
  $SourceLicense = Join-Path $RepoRoot "LICENSE"
  $SourceThirdPartyNotices = Join-Path $RepoRoot "THIRD_PARTY_NOTICES.md"
  $SourceThirdPartyLicenses = Join-Path $RepoRoot "THIRD_PARTY_LICENSES.md"
  $UpdaterSource = Join-Path $RepoRoot "scripts\finish-petri-windows-update.ps1"
} else {
  # Extracted release-package mode remains fail-closed on both package hashes
  # and Authenticode signatures. There is no unsigned-package fallback.
  $SourceBinary = Join-Path $PSScriptRoot "petri.exe"
  $SourceIcon = Join-Path $PSScriptRoot "Petri.ico"
  $SourceLicense = Join-Path $PSScriptRoot "LICENSE"
  $SourceThirdPartyNotices = Join-Path $PSScriptRoot "THIRD_PARTY_NOTICES.md"
  $SourceThirdPartyLicenses = Join-Path $PSScriptRoot "THIRD_PARTY_LICENSES.md"
  $UpdaterSource = Join-Path $PSScriptRoot "finish-petri-windows-update.ps1"
}

foreach ($RequiredPayload in @($SourceBinary, $SourceIcon, $SourceLicense, $SourceThirdPartyNotices, $SourceThirdPartyLicenses, $UpdaterSource)) {
  if (-not (Test-Path -LiteralPath $RequiredPayload -PathType Leaf)) {
    if ($SkipBuild) {
      throw "The existing release build and repository assets are required for -SkipBuild; missing payload: $RequiredPayload"
    }
    throw "The complete Petri release archive is required; missing payload: $RequiredPayload"
  }
}

if (-not $SkipBuild) {
  $PackageHashManifest = Join-Path $PSScriptRoot "SHA256SUMS.txt"
  if (-not (Test-Path -LiteralPath $PackageHashManifest -PathType Leaf)) {
    throw "The packaged installer requires SHA256SUMS.txt. Extract the complete Petri release archive and try again."
  }

  $RequiredPackageNames = @(
    "petri.exe",
    "Petri.ico",
    "LICENSE",
    "THIRD_PARTY_NOTICES.md",
    "THIRD_PARTY_LICENSES.md",
    "install-petri.ps1",
    "finish-petri-windows-update.ps1",
    "README.txt"
  )
  $HashedPackageNames = @{}
  foreach ($Line in Get-Content -LiteralPath $PackageHashManifest) {
    if ([string]::IsNullOrWhiteSpace($Line)) {
      continue
    }
    if ($Line -notmatch '^(?<Hash>[0-9a-fA-F]{64})  (?<Name>[^\\/:*?"<>|]+)$') {
      throw "Invalid SHA256SUMS.txt entry: $Line"
    }
    if ($HashedPackageNames.ContainsKey($Matches.Name)) {
      throw "Duplicate SHA256SUMS.txt entry: $($Matches.Name)"
    }
    $HashedPackageNames[$Matches.Name] = $true
    $PayloadPath = Join-Path $PSScriptRoot $Matches.Name
    if (-not (Test-Path -LiteralPath $PayloadPath -PathType Leaf)) {
      throw "Package payload listed in SHA256SUMS.txt is missing: $($Matches.Name)"
    }
    $ActualHash = (Get-FileHash -LiteralPath $PayloadPath -Algorithm SHA256).Hash
    if ($ActualHash -ine $Matches.Hash) {
      throw "Package integrity check failed for $($Matches.Name)."
    }
  }
  foreach ($RequiredPackageName in $RequiredPackageNames) {
    if (-not $HashedPackageNames.ContainsKey($RequiredPackageName)) {
      throw "SHA256SUMS.txt does not cover required package payload: $RequiredPackageName"
    }
  }

  $InstallerSignature = Get-AuthenticodeSignature -LiteralPath $PSCommandPath
  if ($InstallerSignature.Status -ne [System.Management.Automation.SignatureStatus]::Valid) {
    throw "The packaged Petri installer must have a valid Authenticode signature."
  }
  $UpdaterSignature = Get-AuthenticodeSignature -LiteralPath $UpdaterSource
  if ($UpdaterSignature.Status -ne [System.Management.Automation.SignatureStatus]::Valid) {
    throw "The packaged Petri installer helper must have a valid Authenticode signature."
  }
  $SourceSignature = Get-AuthenticodeSignature -LiteralPath $SourceBinary
  if ($SourceSignature.Status -ne [System.Management.Automation.SignatureStatus]::Valid) {
    throw "Petri must have a valid Authenticode signature before installation."
  }
  $ExpectedPublisherThumbprint = $SourceSignature.SignerCertificate.Thumbprint
  if ([string]::IsNullOrWhiteSpace($ExpectedPublisherThumbprint)) {
    throw "The packaged Petri binary signature does not identify a publisher certificate."
  }
  foreach ($PackageSignature in @($InstallerSignature, $UpdaterSignature)) {
    $PackagePublisherThumbprint = $PackageSignature.SignerCertificate.Thumbprint
    if ([string]::IsNullOrWhiteSpace($PackagePublisherThumbprint) -or
        $PackagePublisherThumbprint -ine $ExpectedPublisherThumbprint) {
      throw "Every executable Petri package payload must be signed by the same publisher certificate."
    }
  }
}

$SourceBinaryHash = (Get-FileHash -LiteralPath $SourceBinary -Algorithm SHA256).Hash
$DeferredInstall = $false
if (-not $InstallRoot) {
  $InstallRoot = Join-Path $env:LOCALAPPDATA "Programs\Petri"
}
$InstallRoot = [System.IO.Path]::GetFullPath($InstallRoot)
New-Item -ItemType Directory -Force -Path $InstallRoot | Out-Null
$AppBinary = Join-Path $InstallRoot "petri.exe"
if ([System.IO.Path]::GetFullPath($SourceBinary) -ieq [System.IO.Path]::GetFullPath($AppBinary)) {
  throw "The Petri install destination must not overwrite its source binary."
}
$AppIcon = Join-Path $InstallRoot "Petri.ico"
$InstalledLicense = Join-Path $InstallRoot "LICENSE"
$InstalledThirdPartyNotices = Join-Path $InstallRoot "THIRD_PARTY_NOTICES.md"
$InstalledThirdPartyLicenses = Join-Path $InstallRoot "THIRD_PARTY_LICENSES.md"
$InstalledUpdater = Join-Path $InstallRoot "finish-petri-windows-update.ps1"
$UpdateStatus = Join-Path $InstallRoot "update-status.json"
Copy-Item -LiteralPath $SourceIcon -Destination $AppIcon -Force
Copy-Item -LiteralPath $SourceLicense -Destination $InstalledLicense -Force
Copy-Item -LiteralPath $SourceThirdPartyNotices -Destination $InstalledThirdPartyNotices -Force
Copy-Item -LiteralPath $SourceThirdPartyLicenses -Destination $InstalledThirdPartyLicenses -Force

$CopiedPayloads = @(
  [pscustomobject]@{ Source = $SourceIcon; Installed = $AppIcon }
  [pscustomobject]@{ Source = $SourceLicense; Installed = $InstalledLicense }
  [pscustomobject]@{ Source = $SourceThirdPartyNotices; Installed = $InstalledThirdPartyNotices }
  [pscustomobject]@{ Source = $SourceThirdPartyLicenses; Installed = $InstalledThirdPartyLicenses }
)
if (-not $SkipBuild) {
  # Source-checkout updates must not replace a previously signed installed
  # helper with the checkout's ordinarily unsigned copy.
  Copy-Item -LiteralPath $UpdaterSource -Destination $InstalledUpdater -Force
  $CopiedPayloads += [pscustomobject]@{ Source = $UpdaterSource; Installed = $InstalledUpdater }
}
foreach ($CopiedPayload in $CopiedPayloads) {
  $SourcePayloadHash = (Get-FileHash -LiteralPath $CopiedPayload.Source -Algorithm SHA256).Hash
  $InstalledPayloadHash = (Get-FileHash -LiteralPath $CopiedPayload.Installed -Algorithm SHA256).Hash
  if ($SourcePayloadHash -ine $InstalledPayloadHash) {
    throw "Installed payload does not match its source: $($CopiedPayload.Installed)"
  }
}

if (-not $SkipBuild) {
  $InstalledUpdaterSignature = Get-AuthenticodeSignature -LiteralPath $InstalledUpdater
  if ($InstalledUpdaterSignature.Status -ne [System.Management.Automation.SignatureStatus]::Valid) {
    throw "The installed Petri helper does not retain a valid Authenticode signature."
  }
  if ($InstalledUpdaterSignature.SignerCertificate.Thumbprint -ine $ExpectedPublisherThumbprint) {
    throw "The installed Petri helper is not signed by the packaged Petri publisher."
  }
}

$StagedBinary = Join-Path $InstallRoot "petri.exe.new"
Copy-Item -LiteralPath $SourceBinary -Destination $StagedBinary -Force
$StagedBinaryHash = (Get-FileHash -LiteralPath $StagedBinary -Algorithm SHA256).Hash
if ($StagedBinaryHash -ine $SourceBinaryHash) {
  throw "The staged Petri binary does not match its source."
}

if ($SkipBuild) {
  # The signed release helper deliberately rejects unsigned local builds. This
  # source-only worker instead pins the exact local build hash through every
  # retry and preserves rollback if Windows has the installed binary locked.
  $SourceUpdateWorker = {
    param(
      [string]$StagedBinary,
      [string]$AppBinary,
      [string]$ExpectedHash,
      [string]$StatusFile,
      [int]$WaitForPid = 0,
      [int]$Attempts = 1,
      [int]$RetryDelayMs = 500
    )

    $ErrorActionPreference = "Stop"

    function Write-SourceUpdateStatus([string]$Status, [string]$Message, [int]$Attempt) {
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

    if ($WaitForPid -gt 0) {
      Write-SourceUpdateStatus "waiting" "Waiting for Petri process $WaitForPid to exit." 0
      Wait-Process -Id $WaitForPid -ErrorAction SilentlyContinue
    }

    $LastFailure = $null
    for ($Attempt = 1; $Attempt -le [Math]::Max(1, $Attempts); $Attempt++) {
      $BackupBinary = "$AppBinary.old.$PID.$Attempt"
      $MovedExistingApp = $false
      try {
        if (-not (Test-Path -LiteralPath $StagedBinary -PathType Leaf)) {
          throw "Missing staged Petri binary: $StagedBinary"
        }
        $CurrentStagedHash = (Get-FileHash -LiteralPath $StagedBinary -Algorithm SHA256).Hash
        if ($CurrentStagedHash -ine $ExpectedHash) {
          throw "The staged Petri binary no longer matches the trusted source-checkout build."
        }
        if (Test-Path -LiteralPath $BackupBinary) {
          Remove-Item -LiteralPath $BackupBinary -Force
        }
        if (Test-Path -LiteralPath $AppBinary) {
          [System.IO.File]::Move($AppBinary, $BackupBinary)
          $MovedExistingApp = $true
        }
        try {
          [System.IO.File]::Move($StagedBinary, $AppBinary)
          $InstalledBinaryHash = (Get-FileHash -LiteralPath $AppBinary -Algorithm SHA256).Hash
          if ($InstalledBinaryHash -ine $ExpectedHash) {
            throw "The installed Petri binary does not match the trusted source-checkout build."
          }
        } catch {
          if (Test-Path -LiteralPath $AppBinary) {
            Remove-Item -LiteralPath $AppBinary -Force -ErrorAction SilentlyContinue
          }
          if ($MovedExistingApp -and (-not (Test-Path -LiteralPath $AppBinary)) -and (Test-Path -LiteralPath $BackupBinary)) {
            [System.IO.File]::Move($BackupBinary, $AppBinary)
          }
          throw
        }
        Remove-Item -LiteralPath $BackupBinary -Force -ErrorAction SilentlyContinue
        Get-ChildItem -LiteralPath (Split-Path -Parent $AppBinary) -Filter "$(Split-Path -Leaf $AppBinary).old.*" -File -ErrorAction SilentlyContinue |
          Remove-Item -Force -ErrorAction SilentlyContinue
        Write-SourceUpdateStatus "installed" "Petri was updated successfully from the source checkout." $Attempt
        return $true
      } catch {
        $LastFailure = $_.Exception.Message
        if ($Attempt -lt [Math]::Max(1, $Attempts)) {
          Start-Sleep -Milliseconds ([Math]::Max(50, $RetryDelayMs))
        }
      }
    }
    Write-SourceUpdateStatus "failed" "Petri could not be replaced: $LastFailure" ([Math]::Max(1, $Attempts))
    return $false
  }

  $InstalledImmediately = & $SourceUpdateWorker `
    -StagedBinary $StagedBinary -AppBinary $AppBinary -ExpectedHash $SourceBinaryHash `
    -StatusFile $UpdateStatus -Attempts 1
  if (-not $InstalledImmediately) {
    $EffectiveWaitForPid = $WaitForPid
    if ($EffectiveWaitForPid -le 0) {
      $RunningPetri = Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
        Where-Object { $_.ExecutablePath -and ([System.IO.Path]::GetFullPath($_.ExecutablePath) -eq $AppBinary) } |
        Select-Object -First 1
      if ($RunningPetri) {
        $EffectiveWaitForPid = [int]$RunningPetri.ProcessId
      }
    }

    $QuotedStaged = "'" + $StagedBinary.Replace("'", "''") + "'"
    $QuotedApp = "'" + $AppBinary.Replace("'", "''") + "'"
    $QuotedHash = "'" + $SourceBinaryHash.Replace("'", "''") + "'"
    $QuotedStatus = "'" + $UpdateStatus.Replace("'", "''") + "'"
    $DeferredCommand = @(
      "`$SourceUpdateWorker = { "
      $SourceUpdateWorker.ToString()
      " }; "
      "`$Installed = & `$SourceUpdateWorker -StagedBinary $QuotedStaged -AppBinary $QuotedApp -ExpectedHash $QuotedHash "
      "-StatusFile $QuotedStatus -WaitForPid $EffectiveWaitForPid -Attempts 120 -RetryDelayMs 500; "
      "if (-not `$Installed) { exit 1 }"
    ) -join ""
    $EncodedCommand = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($DeferredCommand))
    Start-Process -FilePath powershell.exe -ArgumentList @("-NoProfile", "-NonInteractive", "-EncodedCommand", $EncodedCommand) -WindowStyle Hidden
    $DeferredInstall = $true
    $WaitForPid = $EffectiveWaitForPid
  }
} else {
  & powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $InstalledUpdater `
    -StagedBinary $StagedBinary -AppBinary $AppBinary -Attempts 1 -StatusFile $UpdateStatus *> $null
  if ($LASTEXITCODE -ne 0) {
    $EffectiveWaitForPid = $WaitForPid
    if ($EffectiveWaitForPid -le 0) {
      $RunningPetri = Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
        Where-Object { $_.ExecutablePath -and ([System.IO.Path]::GetFullPath($_.ExecutablePath) -eq $AppBinary) } |
        Select-Object -First 1
      if ($RunningPetri) {
        $EffectiveWaitForPid = [int]$RunningPetri.ProcessId
      }
    }

    $QuotedUpdater = "'" + $InstalledUpdater.Replace("'", "''") + "'"
    $QuotedStaged = "'" + $StagedBinary.Replace("'", "''") + "'"
    $QuotedApp = "'" + $AppBinary.Replace("'", "''") + "'"
    $QuotedStatus = "'" + $UpdateStatus.Replace("'", "''") + "'"
    $DeferredCommand = "& $QuotedUpdater -StagedBinary $QuotedStaged -AppBinary $QuotedApp -Attempts 120 -RetryDelayMs 500 -StatusFile $QuotedStatus"
    if ($EffectiveWaitForPid -gt 0) {
      $DeferredCommand += " -WaitForPid $EffectiveWaitForPid"
    }
    $EncodedCommand = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($DeferredCommand))
    Start-Process -FilePath powershell.exe -ArgumentList @("-NoProfile", "-NonInteractive", "-EncodedCommand", $EncodedCommand) -WindowStyle Hidden
    $DeferredInstall = $true
    $WaitForPid = $EffectiveWaitForPid
  }
}

$WorkingDirectory = if ($SkipBuild) { $RepoRoot } else { $InstallRoot }

$CommandDir = Join-Path $HOME ".cargo\bin"
$ShimPath = Join-Path $CommandDir $ShimFileName
New-Item -ItemType Directory -Force -Path $CommandDir | Out-Null
$Shim = "@echo off`r`n`"$AppBinary`" %*`r`n"
Set-Content -LiteralPath $ShimPath -Value $Shim -Encoding ASCII -NoNewline

$LegacyAppLauncher = Join-Path $InstallRoot "petri-app.cmd"
Remove-Item -LiteralPath $LegacyAppLauncher -Force -ErrorAction SilentlyContinue

function CreateShortcut([string]$ShortcutPath) {
  $ShortcutDirectory = Split-Path -Parent $ShortcutPath
  New-Item -ItemType Directory -Force -Path $ShortcutDirectory | Out-Null
  $Shell = New-Object -ComObject WScript.Shell
  $Shortcut = $Shell.CreateShortcut($ShortcutPath)
  $Shortcut.TargetPath = $AppBinary
  $Shortcut.Arguments = "tui"
  $Shortcut.WorkingDirectory = $WorkingDirectory
  $Shortcut.IconLocation = "$AppIcon,0"
  $Shortcut.Description = "Petri - Amoeba market terminal"
  $Shortcut.WindowStyle = 1
  $Shortcut.Save()
}

$StartMenuShortcut = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\$ShortcutName.lnk"
CreateShortcut $StartMenuShortcut

$DesktopShortcut = $null
if (-not $NoDesktopShortcut) {
  $DesktopDirectory = [Environment]::GetFolderPath("Desktop")
  if ($DesktopDirectory) {
    $DesktopShortcut = Join-Path $DesktopDirectory "$ShortcutName.lnk"
    CreateShortcut $DesktopShortcut
  }
}

Write-Host "Installed ${ShortcutName}:"
Write-Host "  App:      $AppBinary"
Write-Host "  Command:  $ShimPath"
Write-Host "  Start:    $StartMenuShortcut"
if ($DesktopShortcut) {
  Write-Host "  Desktop:  $DesktopShortcut"
}
Write-Host ""
if ($DeferredInstall) {
  Write-Host "The release app will finish updating after process $WaitForPid exits."
} else {
  Write-Host "Run $CommandName or click $ShortcutName."
}
