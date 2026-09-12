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
import { packRuntime, readRuntime, sha256 } from './sdk-runtime-archive.mjs';
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const commit = 'a21b324a7a64da87046c7650355b80ea20c47540';
// Distribution commits retain the pinned upstream runtime identity.
const publicSourcePath = path.join(root, 'release/public-sdk-source.json');
const publicSource = fs.existsSync(publicSourcePath) ? JSON.parse(fs.readFileSync(publicSourcePath, 'utf8')) : null;
if (publicSource && (publicSource.upstreamCommit !== commit || !/^[a-f0-9]{40}$/.test(publicSource.commit) || publicSource.repository !== 'https://github.com/amoeba-farm/petri-sdk.git')) throw new Error('Invalid public SDK source identity');
const { values: options, positionals } = parseArgs({ allowPositionals: true, options: {
  verify: { type: 'boolean' }, candidate: { type: 'boolean' },
  'omit-install-archives': { type: 'boolean' }, 'bundle-worker': { type: 'boolean' },
  'compression-level': { type: 'string', default: '9' }, 'no-deduplicate': { type: 'boolean' },
  'verifier-opt-level': { type: 'string' }, 'verifier-lto': { type: 'string' },
} });
if (positionals.length > 1) throw new Error('Expected at most one SDK checkout');
let sdkCheckout = positionals[0] || process.env.PETRI_SDK_CHECKOUT;
const candidate = options.candidate === true;
const level = Number(options['compression-level']);
if (!Number.isInteger(level) || level < 0 || level > 9) throw new Error('Compression level must be 0-9');
if (!candidate && (options['omit-install-archives'] || options['bundle-worker'] || options['verifier-opt-level'] || options['verifier-lto'])) {
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
  if(header.candidate===true||header.schemaVersion!==1||header.sdkCommit!==commit||header.platform!==process.platform||header.arch!==process.arch||!header.files.some(f=>f.path==='node-LICENSE.txt')||
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
run('tar', ['-xf', path.join(stage,'sdk.tar'), '-C', sdk], root);
// Invoke npm through Node, never a shell with caller-controlled arguments.
const npm = process.env.npm_execpath || path.join(path.dirname(process.execPath), 'node_modules/npm/bin/npm-cli.js');
if (!fs.existsSync(npm)) throw new Error('Run using a Node installation with npm available beside it.');
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
const names=[];
function add(name) {
  names.push(name.split(path.sep).join('/'));
}
function walk(directory) {
  for(const entry of fs.readdirSync(path.join(stage,directory),{withFileTypes:true}).sort((a,b)=>a.name.localeCompare(b.name))) {
    if(entry.name==='.bin'||entry.name.endsWith('.map')||entry.name.endsWith('.d.ts'))continue;
    const name=path.join(directory,entry.name);
    if(entry.isSymbolicLink())throw new Error(`Unexpected SDK dependency symlink: ${name}`);
    if(entry.isDirectory())walk(name);else if(entry.isFile())add(name);
  }
}
add(process.platform==='win32'?'node.exe':'node');add('node-LICENSE.txt');add('worker.mjs');
add(verifierName);add('compressed-verifier.json');for(const name of companions)add(name);
for(const directory of ['sdk/dist','sdk/node_modules','sdk/release','sdk/vendor'])walk(directory);
for(const name of fs.readdirSync(sdk).filter(name=>/^(LICENSE|NOTICE|COPYING)(\.|$)/i.test(name)))if(fs.statSync(path.join(sdk,name)).isFile())add(path.join('sdk',name));
add('sdk/package.json');add('sdk/package-lock.json');
const omitted = [];
const included = names.filter(name => {
  const reason = options['omit-install-archives'] && /^sdk\/vendor\/[^/]+\.tgz$/.test(name)
    ? 'installation archive; four-family runtime qualification deferred'
    : bundledInputs.includes(name) ? 'module included in candidate Node bundle; resource qualification deferred' : null;
  if (!reason) return true;
  const bytes = fs.readFileSync(path.join(stage, name));
  omitted.push({ path: name, bytes: bytes.length, sha256: sha256(bytes), reason });
  return false;
});
const metadata = { sdkCommit:commit, platform:process.platform, arch:process.arch, nodeVersion:process.version,
  candidate, compressionLevel:level, deduplicated:!options['no-deduplicate'] };
const result = packRuntime(stage, included, metadata, { level, deduplicate:!options['no-deduplicate'] });
const basename = candidate ? 'petri-sdk-runtime-candidate' : 'petri-sdk-runtime';
fs.writeFileSync(path.join(target, `${basename}.bin`), result.bundle);
fs.writeFileSync(path.join(target, `${basename}.inventory.json`), JSON.stringify({
  ...metadata, qualification:'compiled-only-tests-deferred', bundleBytes:result.bundle.length,
  bundleSha256:sha256(result.bundle), nodeSha256:sha256(fs.readFileSync(process.execPath)),
  sdkLockSha256:sha256(fs.readFileSync(path.join(sdk,'package-lock.json'))),
  cliCommit:execFileSync('git',['rev-parse','HEAD'],{cwd:root,encoding:'utf8',windowsHide:true}).trim(),
  rustc:execFileSync('rustc',['-Vv'],{encoding:'utf8',windowsHide:true}).trim(), verifierEnv,
  omitted, ...result.inventory,
}, null, 2)+'\n');
console.log(`Built ${basename}: ${result.manifest.files.length} files, ${result.bundle.length} bytes, ${os.platform()}/${os.arch()}. No tests run.`);
