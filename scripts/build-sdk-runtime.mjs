// Build an immutable, signer-free SDK runtime inside Petri's target directory.
// Upstream checkouts are read-only. No tests or publish hooks are invoked.
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { parseArgs } from 'node:util';
import { packRuntime, readRuntime, sha256, MAX_GROUP_BYTES } from './sdk-runtime-archive.mjs';
import { RUNTIME_LAYOUT, runtimeInputFiles, selectRuntimeFiles } from './sdk-runtime-files.mjs';
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
// Reject cached archives selected/encoded by older packaging code, even when
// their worker source still matches. No worker execution is needed here.
const runtimeBuildSha256 = sha256(JSON.stringify([
  'build-sdk-runtime.mjs', 'sdk-runtime-files.mjs', 'sdk-runtime-archive.mjs', 'sdk-runtime-candidate.mjs',
  'runtime-tools/package.json', 'runtime-tools/package-lock.json',
].map(name => [name, sha256(fs.readFileSync(path.join(root, 'scripts', name)))])));
const commit = 'a21b324a7a64da87046c7650355b80ea20c47540';
// Distribution commits retain the pinned upstream runtime identity.
const publicSourcePath = path.join(root, 'release/public-sdk-source.json');
const publicSource = fs.existsSync(publicSourcePath) ? JSON.parse(fs.readFileSync(publicSourcePath, 'utf8')) : null;
if (publicSource && (publicSource.upstreamCommit !== commit || !/^[a-f0-9]{40}$/.test(publicSource.commit) || publicSource.repository !== 'https://github.com/amoeba-farm/petri-sdk.git')) throw new Error('Invalid public SDK source identity');
const { values: options, positionals } = parseArgs({ allowPositionals: true, options: {
  verify: { type: 'boolean' }, candidate: { type: 'boolean' },
  'omit-install-archives': { type: 'boolean' }, 'bundle-worker': { type: 'boolean' },
  'compression-level': { type: 'string', default: '9' }, 'no-deduplicate': { type: 'boolean' },
  'group-bytes': { type: 'string', default: '0' }, 'compress-manifest': { type: 'boolean' },
  'verifier-opt-level': { type: 'string' }, 'verifier-lto': { type: 'string' },
} });
if (positionals.length > 1) throw new Error('Expected at most one SDK checkout');
let sdkCheckout = positionals[0] || process.env.PETRI_SDK_CHECKOUT;
const candidate = options.candidate === true;
const level = Number(options['compression-level']);
const groupBytes = Number(options['group-bytes']);
const compressManifest = options['compress-manifest'] === true || groupBytes > 0;
if (!Number.isInteger(groupBytes) || groupBytes < 0 || groupBytes > MAX_GROUP_BYTES) throw new Error('Group bytes must be 0-4194304');
if (!Number.isInteger(level) || level < 0 || level > 9) throw new Error('Compression level must be 0-9');
if (!candidate && (groupBytes || compressManifest || options['bundle-worker'] || options['verifier-opt-level'] || options['verifier-lto'])) {
  throw new Error('Unqualified payload/profile experiments require --candidate');
}
const verifierEnv = {};
for (const [option, allowed, variable] of [
  ['verifier-opt-level', ['2', '3', 's', 'z'], 'CARGO_PROFILE_RELEASE_OPT_LEVEL'],
  ['verifier-lto', ['thin', 'fat'], 'CARGO_PROFILE_RELEASE_LTO'],
]) {
  if (options[option] === undefined) continue;
  if (!allowed.includes(options[option])) throw new Error(`Invalid ${option}`);
  verifierEnv[variable] = options[option];
}
const run = (command, args, cwd) => execFileSync(command, args, { cwd, windowsHide:true, stdio:'inherit' });
const target = path.join(root, 'target');
const companions=['sdk-oracle.mjs','sdk-carry.mjs'];
fs.mkdirSync(target, { recursive:true });
if (options.verify) {
  if (candidate) throw new Error('Candidate runtime qualification is a separate test phase');
  const { manifest: header } = readRuntime(path.join(target,'petri-sdk-runtime.bin'));
  const worker=header.files.find(f=>f.path==='worker.mjs');
  if(header.candidate===true||header.runtimeLayout!==RUNTIME_LAYOUT||header.runtimeBuildSha256!==runtimeBuildSha256||![1,2].includes(header.schemaVersion)||header.sdkCommit!==commit||header.platform!==process.platform||header.arch!==process.arch||!header.files.some(f=>f.path==='node-LICENSE.txt')||
      worker?.sha256!==createHash('sha256').update(fs.readFileSync(path.join(root,'scripts/sdk-worker.mjs'))).digest('hex')||
      companions.some(name=>header.files.find(f=>f.path===name)?.sha256!==createHash('sha256').update(fs.readFileSync(path.join(root,'scripts',name))).digest('hex'))||
      !header.files.some(f=>f.path==='compressed-verifier.json')) {
    throw new Error('SDK runtime is stale or belongs to another build platform. Rebuild it first.');
  }
  console.log('SDK runtime metadata matches this source and build platform.');
  process.exit(0);
}
const stage = fs.mkdtempSync(path.join(target, 'sdk-runtime-'));
const nodeMajor = Number(process.versions.node.split('.')[0]);
if (nodeMajor < 22 || nodeMajor >= 25) throw new Error('The pinned SDK requires Node 22-24');
if(!sdkCheckout){
  sdkCheckout=path.join(stage,'sdk-source');
  run('git',['clone','--no-checkout','--filter=blob:none',publicSource?.repository ?? 'https://github.com/SPACE999978/ameba_sdk.git',sdkCheckout],root);
}
const sdk = path.join(stage, 'sdk');
fs.mkdirSync(sdk);
execFileSync('git', ['archive', '--format=tar', `--output=${path.join(stage,'sdk.tar')}`, publicSource?.commit ?? commit], { cwd:sdkCheckout, windowsHide:true });
const tar = process.platform === 'win32' ? path.join(process.env.SystemRoot || 'C:\\Windows', 'System32', 'tar.exe') : 'tar';
run(tar, ['-xf', path.join(stage,'sdk.tar'), '-C', sdk], root);
// Invoke npm through Node, never a shell with caller-controlled arguments.
const npm = [process.env.npm_execpath,
  path.join(path.dirname(process.execPath), 'node_modules/npm/bin/npm-cli.js'),
  path.resolve(path.dirname(process.execPath), '../lib/node_modules/npm/bin/npm-cli.js'),
].find(candidate => candidate && fs.existsSync(candidate));
if (!npm) throw new Error('Run using a Node installation with npm available.');
run(process.execPath, [npm, 'ci', '--ignore-scripts', '--no-audit', '--no-fund', '--cache',path.join(target,'sdk-npm-cache')], sdk);
run(process.execPath, [path.join(sdk,'node_modules/typescript/bin/tsc'), '-p', 'tsconfig.json'], sdk);
// Build-only, opt-in projection. Runtime resources and external dependencies
// remain inventoried; no candidate is written over the ordinary runtime.
let bundledInputs = [];
if (options['bundle-worker']) {
  const { bundleWorkerSdk } = await import('./sdk-runtime-candidate.mjs');
  bundledInputs = await bundleWorkerSdk(root, stage, sdk);
}
run(process.execPath, [npm, 'prune', '--omit=dev', '--ignore-scripts', '--no-audit', '--no-fund','--cache',path.join(target,'sdk-npm-cache')], sdk);
// Install the locked build-only tracer outside the SDK/runtime payload.
const tracingTools = path.join(stage, 'build-tools');
fs.mkdirSync(tracingTools);
for (const name of ['package.json', 'package-lock.json']) fs.copyFileSync(path.join(root, 'scripts/runtime-tools', name), path.join(tracingTools, name));
run(process.execPath, [npm, 'ci', '--ignore-scripts', '--no-audit', '--no-fund', '--cache', path.join(target, 'sdk-npm-cache')], tracingTools);
const { nodeFileTrace, resolve } = createRequire(path.join(tracingTools, 'package.json'))('@vercel/nft');
// The pinned native package marks the source local-build-unqualified, whereas
// the SDK source-locator helper still expects source-only-unqualified. Resolve
// the package's explicit export without modifying either upstream artifact.
const sdkRequire=createRequire(path.join(sdk,'package.json'));
const sourceRoot=path.dirname(sdkRequire.resolve('@amoeba/spread-release-tools/compressed-evidence-verifier-source'));
const sourceManifest=JSON.parse(fs.readFileSync(path.join(sourceRoot,'source.json'),'utf8'));
if(sourceManifest.schemaVersion!==1||sourceManifest.interfaceSchema!=='ameba-compressed-evidence-v1'||sourceManifest.packageName!=='ameba-current-compressed-evidence-verifier'||sourceManifest.packageVersion!=='0.1.0'||sourceManifest.binaryName!=='ameba-current-compressed-evidence-verifier'||sourceManifest.manifest!=='Cargo.toml'||sourceManifest.lockfile!=='Cargo.lock'||sourceManifest.interface!=='INTERFACE.md'||JSON.stringify(sourceManifest.sources)!=='["src/main.rs"]'||sourceManifest.artifactStatus!=='local-build-unqualified')throw new Error('Pinned native verifier source contract changed');
const source={root:sourceRoot,manifest:sourceManifest,files:['source.json','Cargo.toml','Cargo.lock','INTERFACE.md','src/main.rs'].map(relativePath=>({relativePath,sha256:createHash('sha256').update(fs.readFileSync(path.join(sourceRoot,relativePath))).digest('hex')}))};
const verifierTarget=path.join(target,'sdk-compressed-verifier');
execFileSync('cargo',['build','--release','--locked','--manifest-path',path.join(source.root,'Cargo.toml'),'--target-dir',verifierTarget,'-j1'],{
  cwd:root, windowsHide:true, stdio:'inherit', env:{...process.env,...verifierEnv},
});
const verifierName=process.platform==='win32'?'compressed-verifier.exe':'compressed-verifier';
fs.copyFileSync(path.join(verifierTarget,'release',source.manifest.binaryName+(process.platform==='win32'?'.exe':'')),path.join(stage,verifierName));
fs.writeFileSync(path.join(stage,'compressed-verifier.json'),JSON.stringify({sdkCommit:commit,source:source.files.map(f=>({path:f.relativePath,sha256:f.sha256})),sha256:createHash('sha256').update(fs.readFileSync(path.join(stage,verifierName))).digest('hex'),qualification:'compiled-only-tests-deferred'}));
fs.copyFileSync(process.execPath, path.join(stage, process.platform==='win32'?'node.exe':'node'));
const nodeLicense=await fetch(`https://raw.githubusercontent.com/nodejs/node/${process.version}/LICENSE`,{redirect:'error',signal:AbortSignal.timeout(20000)});
if(!nodeLicense.ok)throw new Error('Could not obtain the exact bundled Node version license');
const licenseParts=[];let licenseSize=0;
for await(const chunk of nodeLicense.body){licenseSize+=chunk.length;if(licenseSize>2*1024*1024)throw new Error('Node license size is invalid');licenseParts.push(chunk);}
fs.writeFileSync(path.join(stage,'node-LICENSE.txt'),Buffer.concat(licenseParts));
fs.copyFileSync(path.join(root,'scripts/sdk-worker.mjs'),path.join(stage,'worker.mjs'));
for(const name of companions)fs.copyFileSync(path.join(root,'scripts',name),path.join(stage,name));
// Trace computed imports, require/exports branches, native loaders and assets.
// Unknown inputs fail closed; there is no whole-tree fallback.
const selection = await selectRuntimeFiles(stage, runtimeInputFiles(stage), { nodeFileTrace, resolve });
const omitted = [...selection.omitted];
const included = selection.included.filter(name => {
  const reason = bundledInputs.includes(name) ? 'module included in candidate Node bundle; resource qualification deferred' : null;
  if (!reason) return true;
  const bytes = fs.readFileSync(path.join(stage, name));
  omitted.push({ path: name, bytes: bytes.length, sha256: sha256(bytes), reason });
  return false;
});
included.push(process.platform==='win32'?'node.exe':'node', 'node-LICENSE.txt',
  'worker.mjs', verifierName, 'compressed-verifier.json', ...companions);
const metadata = { sdkCommit:commit, platform:process.platform, arch:process.arch, nodeVersion:process.version,
  runtimeLayout:RUNTIME_LAYOUT, runtimeBuildSha256,
  candidate, compressionLevel:level, deduplicated:!options['no-deduplicate'],
  compressionGroupBytes:groupBytes, compressedManifest:compressManifest };
const result = packRuntime(stage, included, metadata, {
  level, deduplicate:!options['no-deduplicate'], groupBytes, compressManifest,
});
const basename = candidate ? 'petri-sdk-runtime-candidate' : 'petri-sdk-runtime';
fs.writeFileSync(path.join(target, `${basename}.bin`), result.bundle);
fs.writeFileSync(path.join(target, `${basename}.inventory.json`), JSON.stringify({
  ...metadata, qualification:'compiled-only-tests-deferred', bundleBytes:result.bundle.length,
  bundleSha256:sha256(result.bundle), nodeSha256:sha256(fs.readFileSync(process.execPath)),
  sdkLockSha256:sha256(fs.readFileSync(path.join(sdk,'package-lock.json'))),
  cliCommit:execFileSync('git',['rev-parse','HEAD'],{cwd:root,encoding:'utf8',windowsHide:true}).trim(),
  rustc:execFileSync('rustc',['-Vv'],{encoding:'utf8',windowsHide:true}).trim(), verifierEnv,
  dependencyClosure:selection.dependencyClosure, omitted, ...result.inventory,
}, null, 2)+'\n');
console.log(`Built ${basename}: ${result.manifest.files.length} files, ${result.bundle.length} bytes, ${os.platform()}/${os.arch()}. No tests run.`);
