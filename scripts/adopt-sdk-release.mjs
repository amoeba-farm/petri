// Mechanical release adoption. Reads immutable Git objects; writes only this repo.
// Historical fixtures, audit receipts, and tests are deliberately not rewritten.
import { readFileSync, writeFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const [sdkPath, leanPath, sdk, lean] = process.argv.slice(2);
if (!sdkPath || !leanPath || !/^[a-f0-9]{40}$/.test(sdk ?? '') || !/^[a-f0-9]{40}$/.test(lean ?? ''))
  throw new Error('Usage: node scripts/adopt-sdk-release.mjs <sdk-checkout> <lean-checkout> <sdk-commit> <lean-commit>');
const gitText = (cwd, object) => execFileSync('git', ['show', object], { cwd, encoding: 'utf8', windowsHide: true, maxBuffer: 16 * 1024 * 1024 });
const read = file => readFileSync(path.join(root, file), 'utf8');
const write = (file, text) => writeFileSync(path.join(root, file), text);
const sdkJson = file => JSON.parse(gitText(sdkPath, `${sdk}:${file}`));
const selected = sdkJson('release/current-deployment.v1.json');
const provenance = sdkJson('vendor/GOVERNED_RUNTIME_PROVENANCE.json');
const d = selected.deployment;
if (d.sourceCommit !== provenance.sourceCommit || d.programId !== '2jVQSPny9eFoaG1ZWoJVAezQ5VgqJtF8rQCQXMktuBVw') throw new Error('Inconsistent release');
const status = JSON.parse(read('release/current-governance-status.json'));
const oldSdk = status.sdk.commit;
const old = status.liveDeployment;
const replacements = new Map([
  [oldSdk, sdk], [old.releaseLabel, d.releaseLabel], [old.liveReadProfileId, d.liveReadProfileId],
  [old.programDataAccountSha256, d.programDataAccountSha256],
  [old.programDataPayloadSha256, d.programDataPayloadSha256],
  [String(old.programDataAccountBytes), String(d.programDataAccountBytes)],
  [String(old.programDataPayloadBytes), String(d.programDataPayloadBytes)],
  [String(old.programDataSlot), String(d.programDataSlot)],
  ['3855d707429c0073d1119fb4503df084239f078c3b8a2849138c428fb0bb9d45', provenance.instructionManifestSourceSha256],
]);
// Update runtime code only before inline historical test modules.
for (const file of ['Cargo.toml', 'src/current_release.rs', 'src/agent_protocol.rs', 'src/lab/writers.rs', 'src/writer_output.rs',
  'scripts/build-sdk-runtime.mjs', 'scripts/sdk-worker.mjs',
  'docs/html/source-map.html', 'docs/mcp-agent-protocol.md', 'docs/petri-production-agent-guide.md',
  'release/PUBLIC_RELEASE_PROCESS.md', 'scripts/check-current-governance-release.mjs', 'scripts/check-petri-packaging.mjs']) {
  let text = read(file);
  const boundary = file.endsWith('.rs') ? text.indexOf('#[cfg(test)]') : -1;
  let source = boundary < 0 ? text : text.slice(0, boundary);
  const tail = boundary < 0 ? '' : text.slice(boundary);
  for (const [from, to] of replacements) if (from && from !== to) source = source.replaceAll(from, to);
  if (file === 'src/current_release.rs') source = source.replace(/(REVIEWED_BRIDGE_SOURCE_COMMIT: &str = ")[^"]+/, `$1${d.artifactSourceCommit}`);
  if (file === 'scripts/check-current-governance-release.mjs') source = source.replaceAll(old.sourceCommit, d.artifactSourceCommit).replaceAll(String(old.minimumContextSlot), String(d.minimumContextSlot));
  write(file, source + tail);
}
status.preparedDate = '2026-09-12';
status.packageSource.status = 'local-source-unqualified';
status.sdk.commit = sdk;
status.sourceHeads.lean = lean;
status.sourceHeads.spreadTools = d.sourceCommit;
// Petri's sourceCommit denotes artifact source; SDK distinguishes native publication.
status.liveDeployment = { ...old, ...d, sourceCommit: d.artifactSourceCommit, identityGeneration: 3 };
status.liveGovernance = selected.governance;
status.writeRelease.governedSpreadReleaseCommit = d.sourceCommit;
status.writeRelease.governedInstructionManifestSha256 = provenance.instructionManifestSourceSha256;
status.runtimePermission = { available: false, observationOnly: true, observedAt: d.observedAt,
  gateStatus: selected.governance.pinnedObservation.status, businessState: 'live-action-specific-revalidation-required', revalidateBeforeEveryWrite: true };
status.finalVerification = { status: 'release-adoption-awaiting-verification', passed: null, checks: [], broadIntegrationReruns: false };
write('release/current-governance-status.json', JSON.stringify(status, null, 2) + '\n');
const vocabularySource = gitText(leanPath, `${lean}:Ameba/Domain/CollectiveActionMask.lean`);
const wireNames = vocabularySource.split('def CollectiveActionKind.wireName :')[1]?.split('def allCollectiveActionKinds')[0];
if (!wireNames) throw new Error('Action wire-name definition is missing');
const names = [...wireNames.matchAll(/\| \.(\w+) => "([a-z_]+)"/g)].map(match => match[2]);
if (names.length !== 22 || names.includes('seat_approve')) throw new Error('Unexpected action contract');
write('schemas/collective-actions.v1.json', JSON.stringify({ schemaVersion: 1, semanticAbiVersion: 'collective-operations-v1', leanCommit: lean,
  sdkCommit: sdk, vocabularySha256: createHash('sha256').update(JSON.stringify(names)).digest('hex'), actions: names }, null, 2) + '\n');
// This is identity/planned catalog data, never a live read or action fallback.
// The Edge catalog is a presentation catalog and omits Petri's required Light
// and mint identities. Preserve that separately attested identity inventory.
const catalogSource = gitText(root, '94c5b8014a70eb46408d4304eea237331640f105:config/options_catalog.json');
const catalog = JSON.parse(catalogSource);
const edgeCatalog = JSON.parse(gitText(leanPath, `${lean}:edge/config/options_catalog.json`));
if (catalog.programId !== d.programId || edgeCatalog.programId !== d.programId || catalog.networkIdentity.genesisHash !== d.genesisHash) throw new Error('Foreign catalog');
catalog.deploymentReferences = {sdkCommit:sdk, artifactSourceCommit:d.artifactSourceCommit, nativeSourceCommit:d.sourceCommit, programDataAccountSha256:d.programDataAccountSha256};
const references = JSON.stringify(catalog.deploymentReferences, null, 4).split('\n').map((line, i) => i ? `    ${line}` : line).join('\n');
write('config/options_catalog.json', catalogSource.trimEnd().replace(/\r?\n}$/, `,\n    "deploymentReferences": ${references}\n}\n`));
console.log(`Adopted SDK ${sdk}; artifact ${d.artifactSourceCommit}; native publication ${d.sourceCommit}. Tests untouched.`);
