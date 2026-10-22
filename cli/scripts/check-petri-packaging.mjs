import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

for (const asset of ["../assets/petri-icon.png", "../assets/Petri.ico", "../assets/Petri.icns"]) {
  try {
    if (readFileSync(new URL(asset, import.meta.url)).length === 0) {
      throw new Error("asset is empty");
    }
  } catch (error) {
    throw new Error(`missing Petri application icon: ${asset}`, { cause: error });
  }
}

const nativeUpdaterSource = readFileSync(new URL("../src/workspace_update.rs", import.meta.url), "utf8");
if (nativeUpdaterSource.includes("build_debug_binary_after_release_lock")) {
  throw new Error("native release updates must not fall back to a debug build");
}

const buildSource = readFileSync(new URL("../build.rs", import.meta.url), "utf8");
const cargoSource = readFileSync(new URL("../Cargo.toml", import.meta.url), "utf8");
const cargoLockSource = readFileSync(new URL("../Cargo.lock", import.meta.url), "utf8");
const thirdPartyNotices = readFileSync(new URL("../THIRD_PARTY_NOTICES.md", import.meta.url), "utf8");
const launcherSource = readFileSync(new URL("petri-launcher.mjs", import.meta.url), "utf8");
const macosPackageSource = readFileSync(new URL("package-petri-macos.sh", import.meta.url), "utf8");
const credentialScanSource = readFileSync(new URL("credential-scan.mjs", import.meta.url), "utf8");
const releaseSecretCheckSource = readFileSync(new URL("check-release-secrets.mjs", import.meta.url), "utf8");
const windowsInstallerSource = readFileSync(new URL("install-petri.ps1", import.meta.url), "utf8");
const windowsUpdaterSource = readFileSync(new URL("finish-petri-windows-update.ps1", import.meta.url), "utf8");
const windowsPackageSource = readFileSync(new URL("package-petri-windows.ps1", import.meta.url), "utf8");
const macosInstallerSource = readFileSync(new URL("install-petri-macos.sh", import.meta.url), "utf8");
const previewPackageSource = readFileSync(new URL("package-petri-preview.mjs", import.meta.url), "utf8");
const previewMacLauncher = readFileSync(new URL("petri-macos-launcher.c", import.meta.url), "utf8");
const releaseUpdatePlatformSource = readFileSync(new URL("../src/release_update/platform.rs", import.meta.url), "utf8");
for (const contract of ["'--json','update','info'", "PETRI_UPDATE_CHANNEL=preview", "petri-update.json", "scripts/petri-macos-launcher.c", "'--norsrc'", "'--noextattr'", "path.join(roundtrip,stem,'Petri.app')"]) {
  if (!previewPackageSource.includes(contract)) throw new Error(`Preview packaging is missing standalone update contract: ${contract}`);
}
if (!previewMacLauncher.includes('execl("/usr/bin/open"')) {
  throw new Error('Preview updates require the native Mac launcher.');
}
for (const contract of ["ShellExecuteExW", "SW_SHOWNORMAL", "SEE_MASK_NOCLOSEPROCESS"]) {
  if (!releaseUpdatePlatformSource.includes(contract)) {
    throw new Error(`Windows updater restart must use shell activation with a fresh console: ${contract}`);
  }
}
if (/creation_flags\(0x0000_0010\)/.test(releaseUpdatePlatformSource)) {
  throw new Error("Windows updater restart must not inherit the hidden helper's NUL terminal handles");
}

if (process.platform === "win32") {
  for (const scriptName of [
    "install-petri.ps1",
    "finish-petri-windows-update.ps1",
    "package-petri-windows.ps1",
  ]) {
    const scriptPath = fileURLToPath(new URL(scriptName, import.meta.url));
    const parserCommand = [
      "$tokens = $null",
      "$errors = $null",
      "[System.Management.Automation.Language.Parser]::ParseFile($env:PETRI_POWERSHELL_PATH, [ref]$tokens, [ref]$errors) | Out-Null",
      'if ($errors.Count -gt 0) { $errors | ForEach-Object { [Console]::Error.WriteLine($_.Message) }; exit 1 }',
    ].join("; ");
    const parseResult = spawnSync(
      "powershell.exe",
      ["-NoProfile", "-NonInteractive", "-Command", parserCommand],
      {
        encoding: "utf8",
        windowsHide: true,
        env: { ...process.env, PETRI_POWERSHELL_PATH: scriptPath },
      },
    );
    if (parseResult.status !== 0) {
      throw new Error(`Windows PowerShell rejected scripts/${scriptName}:\n${parseResult.stderr || parseResult.stdout}`);
    }
  }
}
for (const governed of ["ramx", "nandx"]) {
  readFileSync(new URL(`../../oracle/governance/${governed}_product_sku_manifest.v1.json`, import.meta.url));
}
// The commodity buckets of the chip months come from ameba-commodity-spec,
// which compiles in its own governed documents.
readFileSync(new URL("../../oracle/crates/commodity-spec/Cargo.toml", import.meta.url));

const lockedPackages = new Set(
  cargoLockSource
    .split("[[package]]")
    .slice(1)
    .map((block) => {
      const name = block.match(/^name = "([^"]+)"$/m)?.[1];
      const version = block.match(/^version = "([^"]+)"$/m)?.[1];
      return name && version ? `${name}@${version}` : null;
    })
    .filter(
      (entry) =>
        entry &&
        // First-party Amoeba crates (this package, the SDK, the futures and
        // oracle program crates) are not third-party components.
        !/^(?:amoeba|ameba)(?:[-_@])/.test(entry),
    ),
);
const noticedPackages = new Set(
  [...thirdPartyNotices.matchAll(/^\| ([^|]+) \| ([^|]+) \|/gm)]
    .map((match) => `${match[1].trim()}@${match[2].trim()}`)
    .filter((entry) => entry !== "Component@Version" && entry !== "---@---"),
);
const missingNotices = [...lockedPackages].filter((entry) => !noticedPackages.has(entry));
const staleNotices = [...noticedPackages].filter((entry) => !lockedPackages.has(entry));
if (missingNotices.length > 0 || staleNotices.length > 0) {
  throw new Error(
    `THIRD_PARTY_NOTICES.md must exactly match Cargo.lock (missing: ${missingNotices.join(", ") || "none"}; stale: ${staleNotices.join(", ") || "none"})`,
  );
}
// In the Amoeba monorepo the first-party crates are path dependencies on the
// tree itself (../sdk, ../oracle), never git pins of the old repositories.
if (/git\s*=\s*"https:\/\/github\.com\/spacematter78\//.test(cargoSource)) {
  throw new Error("Cargo.toml must not pin first-party crates by git; use the monorepo's path dependencies");
}
if (/path\s*=\s*"\.\.\//.test(launcherSource) || launcherSource.includes('"../oracle') || launcherSource.includes('"../futures')) {
  throw new Error("scripts/petri-launcher.mjs must not require a sibling checkout");
}
for (const requiredDependencyContract of [
  'package = "ameba-sdk", path = "../sdk"',
  'ameba_oracle = { path = "../oracle/programs/ameba_oracle", features = ["no-entrypoint"] }',
  'ameba-commodity-spec = { path = "../oracle/crates/commodity-spec" }',
  'exclude = ["tools/petri-conpty-harness"]',
]) {
  if (!cargoSource.includes(requiredDependencyContract)) {
    throw new Error(`Cargo.toml is missing a monorepo dependency contract: ${requiredDependencyContract}`);
  }
}
if (cargoSource.includes("localnet-dev-authority")) {
  throw new Error("Petri must not build the oracle's localnet dev profile; builders take the deployment's program id");
}
for (const [sourceName, source, requiredContract] of [
  ["credential scanner", credentialScanSource, "f767596811f8"],
  ["credential scan check", releaseSecretCheckSource, "assertCredentialScannerSelfTest();"],
]) {
  if (!source.includes(requiredContract)) {
    throw new Error(`${sourceName} is missing the compromised-credential rejection contract`);
  }
}
if (!buildSource.includes('set_icon("assets/Petri.ico")') || !cargoSource.includes('winresource = "0.1.31"')) {
  throw new Error("Windows petri.exe must embed the Petri icon resource");
}
if (!buildSource.includes('"--git-path"') || !buildSource.includes('"symbolic-ref", "-q", "HEAD"')) {
  throw new Error("build provenance must watch the real worktree HEAD and branch-ref paths");
}
for (const macosVersionContract of [
  'PACKAGE_VERSION="$(awk',
  '/^\\[package\\]$/',
  'BUNDLE_VERSION="${PACKAGE_VERSION%%[-+]*}"',
  '! "$BUNDLE_VERSION" =~ ^[0-9]+\\.[0-9]+\\.[0-9]+$',
  '<key>CFBundleShortVersionString</key><string>$BUNDLE_VERSION</string>',
  '<key>CFBundleVersion</key><string>$BUNDLE_VERSION</string>',
]) {
  if (!macosPackageSource.includes(macosVersionContract)) {
    throw new Error(`macOS package metadata is missing dynamic version contract: ${macosVersionContract}`);
  }
}
const mainSource = readFileSync(new URL("../src/main.rs", import.meta.url), "utf8");
let windowsIdentitySource;
try {
  windowsIdentitySource = readFileSync(new URL("../src/windows_app_identity.rs", import.meta.url), "utf8");
} catch (error) {
  throw new Error("missing Windows runtime application identity hook", { cause: error });
}
for (const runtimeContract of [
  "mod windows_app_identity;",
  "windows_app_identity::apply();",
  "WM_SETICON",
  "SendMessageW",
  "GetConsoleWindow",
]) {
  if (!mainSource.includes(runtimeContract) && !windowsIdentitySource.includes(runtimeContract)) {
    throw new Error(`missing Windows runtime taskbar icon contract: ${runtimeContract}`);
  }
}
for (const requiredUpdaterSource of [
  '"-WaitForPid"',
  "std::process::id()",
  "PETRI_TRUSTED_UPDATE_COMMIT",
  "ensure_canonical_origin",
  '["fetch", "--prune", "origin"]',
  '["merge", "--ff-only", &trusted_commit]',
] ) {
  if (!nativeUpdaterSource.includes(requiredUpdaterSource)) {
    throw new Error(`native updater is missing safe installed-app replacement: ${requiredUpdaterSource}`);
  }
}
for (const requiredLauncherTrust of [
  "PETRI_TRUSTED_UPDATE_COMMIT",
  "ensureCanonicalOrigin",
  '["fetch", "--prune", "origin"]',
  '["merge", "--ff-only", trustedCommit]',
]) {
  if (!launcherSource.includes(requiredLauncherTrust)) {
    throw new Error(`JavaScript updater is missing release provenance contract: ${requiredLauncherTrust}`);
  }
}
if (launcherSource.includes('["merge", "--ff-only", "origin/HEAD"]')) {
  throw new Error("JavaScript updater must never merge an untrusted mutable remote ref");
}

for (const sourceCheckoutContract of [
  "[switch]$SkipBuild",
  "$CheckoutInstaller",
  "-SkipBuild must be run from the source checkout's scripts\\install-petri.ps1",
  'Join-Path $RepoRoot "target\\release\\petri.exe"',
  'Join-Path $RepoRoot "assets\\Petri.ico"',
  'Join-Path $RepoRoot "LICENSE"',
  'Join-Path $RepoRoot "THIRD_PARTY_NOTICES.md"',
  'Join-Path $RepoRoot "THIRD_PARTY_LICENSES.md"',
  'Join-Path $RepoRoot "scripts\\finish-petri-windows-update.ps1"',
  "@($SourceBinary, $SourceIcon, $SourceLicense, $SourceThirdPartyNotices, $SourceThirdPartyLicenses, $UpdaterSource)",
  "Copy-Item -LiteralPath $SourceThirdPartyLicenses -Destination $InstalledThirdPartyLicenses -Force",
  "[pscustomobject]@{ Source = $SourceThirdPartyLicenses; Installed = $InstalledThirdPartyLicenses }",
  "$SourceUpdateWorker",
  "$SourceBinaryHash",
  "must not overwrite its source binary",
  "$CurrentStagedHash",
  "$InstalledBinaryHash",
  "trusted source-checkout build",
  "-WaitForPid $EffectiveWaitForPid -Attempts 120",
]) {
  if (!windowsInstallerSource.includes(sourceCheckoutContract)) {
    throw new Error(`Windows source-checkout installer is missing -SkipBuild contract: ${sourceCheckoutContract}`);
  }
}
for (const packagedReleaseContract of [
  "if (-not $SkipBuild) {",
  'Join-Path $PSScriptRoot "SHA256SUMS.txt"',
  'Join-Path $PSScriptRoot "THIRD_PARTY_LICENSES.md"',
  '"THIRD_PARTY_LICENSES.md",',
  "$RequiredPackageNames",
  "$HashedPackageNames.ContainsKey",
  "Duplicate SHA256SUMS.txt entry",
  "Get-AuthenticodeSignature -LiteralPath $PSCommandPath",
  "Get-AuthenticodeSignature -LiteralPath $UpdaterSource",
  "Get-AuthenticodeSignature -LiteralPath $SourceBinary",
  "Get-AuthenticodeSignature -LiteralPath $InstalledUpdater",
  "$ExpectedPublisherThumbprint",
  "$PackageSignature.SignerCertificate.Thumbprint",
  "& powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $InstalledUpdater",
]) {
  if (!windowsInstallerSource.includes(packagedReleaseContract)) {
    throw new Error(`Windows packaged installer is missing integrity contract: ${packagedReleaseContract}`);
  }
}
for (const [name, source, requiredContracts] of [
  ["finish-petri-windows-update.ps1", windowsUpdaterSource, ["$ExpectedPublisherThumbprint", "$ExpectedBinaryHash", "$StagedSignature.SignerCertificate.Thumbprint", "$InstalledSignature.SignerCertificate.Thumbprint", "Get-FileHash -LiteralPath $AppBinary"]],
  ["package-petri-windows.ps1", windowsPackageSource, ["$PublisherThumbprint", "$ReleaseScriptSignatures", "$PackagedScriptSignature.SignerCertificate.Thumbprint"]],
]) {
  for (const contract of requiredContracts) {
    if (!source.includes(contract)) {
      throw new Error(`${name} is missing same-publisher Authenticode enforcement: ${contract}`);
    }
  }
}
const signedInstallerCopy = 'cp "$SCRIPT_DIR/install-petri-macos.sh" "$RESOURCES_DIR/install-petri-macos.sh"';
if (
  !macosPackageSource.includes(signedInstallerCopy)
  || macosPackageSource.indexOf(signedInstallerCopy) > macosPackageSource.indexOf("/usr/bin/codesign --force")
  || macosPackageSource.includes('"$PACKAGE_ROOT/install-petri-macos.sh"')
  || !macosInstallerSource.includes('SOURCE_APP="$(cd "$SCRIPT_DIR/../.." && pwd)"')
) {
  throw new Error("macOS installer must live inside and verify the signed Petri.app bundle");
}
for (const forbiddenInstallerContract of [
  "[switch]$Dev",
  "Petri Dev",
  "petri-dev",
  "target\\debug",
  "[string]$SourceBinary",
  "SkipSignature",
  "AllowUnsigned",
]) {
  if (windowsInstallerSource.includes(forbiddenInstallerContract)) {
    throw new Error(`Windows installer must not expose development or signature-bypass mode: ${forbiddenInstallerContract}`);
  }
}
const installedUpdaterCopies = windowsInstallerSource.match(/Copy-Item -LiteralPath \$UpdaterSource -Destination \$InstalledUpdater/g) ?? [];
if (installedUpdaterCopies.length !== 1 || !/if \(-not \$SkipBuild\) \{\s+# Source-checkout updates must not replace[\s\S]*?Copy-Item -LiteralPath \$UpdaterSource -Destination \$InstalledUpdater/.test(windowsInstallerSource)) {
  throw new Error("Windows source-checkout updates must not overwrite the installed signed updater helper");
}

const requiredFiles = new Map([
  ["install-petri.ps1", ["CreateShortcut", "Petri.ico", "THIRD_PARTY_NOTICES.md", "THIRD_PARTY_LICENSES.md", "$SourceThirdPartyLicenses", "$InstalledThirdPartyLicenses", "finish-petri-windows-update.ps1", "update-status.json", "WaitForPid", "EncodedCommand", "WindowStyle Hidden", "Get-AuthenticodeSignature", "Get-FileHash", "$Shortcut.TargetPath = $AppBinary", "$Shortcut.Arguments = \"tui\""]],
  ["finish-petri-windows-update.ps1", ["[System.IO.File]::Move($AppBinary, $BackupBinary)", "[System.IO.File]::Move($StagedBinary, $AppBinary)", "Write-UpdateStatus \"installed\"", "Wait-Process", "RetryDelayMs", "Get-AuthenticodeSignature"]],
  ["package-petri-windows.ps1", ["target\\release\\petri.exe", "Petri-windows", "install-petri.ps1", "finish-petri-windows-update.ps1", "assets\\Petri.ico", "BuildOnly", "Get-AuthenticodeSignature", "THIRD_PARTY_NOTICES.md", "THIRD_PARTY_LICENSES.md", "Get-FileHash"]],
  ["package-petri-macos.sh", ["Contents/MacOS", "Contents/Resources", "Info.plist", "petri.command", "open -a Terminal", "Petri.icns", "CFBundleIconFile", " tui", "PETRI_MACOS_SIGNING_IDENTITY", "notarytool submit", "THIRD_PARTY_NOTICES.md", "THIRD_PARTY_LICENSES.md", "shasum -a 256"]],
  ["install-petri-macos.sh", ["$HOME/Applications/Petri.app", "$HOME/.local/bin/$COMMAND_NAME"]],
]);

for (const [name, requiredSnippets] of requiredFiles) {
  let source;
  try {
    source = readFileSync(new URL(name, import.meta.url), "utf8");
  } catch (error) {
    throw new Error(`missing Petri app packaging script: scripts/${name}`, { cause: error });
  }
  for (const snippet of requiredSnippets) {
    if (!source.includes(snippet)) {
      throw new Error(`scripts/${name} is missing packaging contract: ${snippet}`);
    }
  }
}

const readme = readFileSync(new URL("../README.md", import.meta.url), "utf8");
const commandGuide = readFileSync(new URL("../docs/amoeba-cli-command-guide.md", import.meta.url), "utf8");
const bundledCliGuide = readFileSync(new URL("../assets/gitbook/docs/getting-started/02-cli.md", import.meta.url), "utf8");
for (const installCopy of [
  "## Package for Windows",
  "## Package for macOS",
  "scripts/package-petri-windows.ps1",
  "scripts/package-petri-macos.sh",
  "THIRD_PARTY_NOTICES.md",
  "THIRD_PARTY_LICENSES.md",
]) {
  if (!readme.includes(installCopy)) {
    throw new Error(`README is missing application installation guidance: ${installCopy}`);
  }
}

for (const [guideName, guideSource] of [
  ["README", readme],
  ["command guide", commandGuide],
  ["bundled CLI guide", bundledCliGuide],
]) {
  for (const unpublishedDownloadUrl of [
    "https://github.com/SPACE999978/amoeba-cli/releases/latest/download/Petri-macos-arm64.zip",
    "https://github.com/SPACE999978/amoeba-cli/releases/latest/download/Petri-windows-x64.zip",
  ]) {
    if (guideSource.includes(unpublishedDownloadUrl)) {
      throw new Error(`${guideName} advertises an unpublished Petri download: ${unpublishedDownloadUrl}`);
    }
  }
  for (const sourceInstallContract of [
    "## Install from source",
    "cargo build --release --locked --bin petri",
    "not publisher-signed or Apple-notarized",
  ]) {
    const normalizedGuideSource = guideSource.replace(/\s+/g, " ");
    const normalizedContract = sourceInstallContract.replace(/\s+/g, " ");
    if (!normalizedGuideSource.includes(normalizedContract)) {
      throw new Error(`${guideName} is missing source-only release guidance: ${sourceInstallContract}`);
    }
  }
}

// The updater trusts one release repository (build.rs, overridable at build
// time with PETRI_RELEASE_REPOSITORY, as the release workflow does); the
// README's download and installer links must name the same one.
const buildScript = readFileSync(new URL("../build.rs", import.meta.url), "utf8");
const defaultRepository = /const DEFAULT_RELEASE_REPOSITORY: &str = "([^"]+)";/.exec(buildScript)?.[1];
if (!defaultRepository) {
  throw new Error("build.rs is missing DEFAULT_RELEASE_REPOSITORY");
}
const readmeRepositories = new Set(
  [...readme.matchAll(/https:\/\/(?:github\.com|raw\.githubusercontent\.com)\/([A-Za-z0-9-]+\/[A-Za-z0-9._-]+)\/(?:releases|main)\//g)].map((m) => m[1]),
);
if (readmeRepositories.size !== 1 || !readmeRepositories.has(defaultRepository)) {
  throw new Error(
    `README release links name ${[...readmeRepositories].join(", ") || "no repository"}, but the updater trusts ${defaultRepository}`,
  );
}

console.log("Petri packaging checks passed.");
