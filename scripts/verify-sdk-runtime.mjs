// Small offline release gate: extraction, four worker-family rejection paths,
// lazy SDK resources/native imports, and the verifier's fail-closed interface.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {spawnSync} from 'node:child_process';
import {readRuntime,runtimeFiles} from './sdk-runtime-archive.mjs';
const runtime=readRuntime(process.argv[2] ?? 'target/petri-sdk-runtime.bin');
const {manifest}=runtime;
const stage=fs.mkdtempSync(path.join(os.tmpdir(),'petri-runtime-smoke-'));
for(const {entry,bytes} of runtimeFiles(runtime)){
  const file=path.join(stage,entry.path);fs.mkdirSync(path.dirname(file),{recursive:true});fs.writeFileSync(file,bytes);
}
const node=path.join(stage,process.platform==='win32'?'node.exe':'node');
fs.chmodSync(node,0o700);
function run(args, input) {
  const offline='data:text/javascript,'+encodeURIComponent("globalThis.fetch=async()=>{throw Object.assign(new Error('OFFLINE_RPC_BLOCKED'),{code:'OFFLINE_RPC_BLOCKED'})}");
  const result=spawnSync(node,['--import',offline,...args],{cwd:stage,input,encoding:'utf8',windowsHide:true,timeout:30000,
    env:{...process.env,NODE_OPTIONS:'',NODE_PATH:''},maxBuffer:2*1024*1024});
  if(result.error||result.signal)throw result.error??new Error(`SDK probe terminated: ${result.signal}`);
  return result;
}
const owner='11111111111111111111111111111111';
const cases=[
  {schemaVersion:0},
  ...['collateral','oracle','liquidity','carry'].map(family=>({schemaVersion:1,family,owner,
    backend:'http://127.0.0.1:1',request:{actionType:'not_an_action',unexpected:true},prepared:{}})),
];
const errors=[];
for(const input of cases){
  const result=run([path.join(stage,'worker.mjs')],JSON.stringify(input));
  let response;try{response=JSON.parse(result.stdout)}catch{throw new Error('Packaged SDK worker failed to start: '+(result.stderr||result.stdout));}
  assert.equal(result.status,1);assert.equal(response.ok,false);
  assert.equal(typeof response.error?.message,'string');
  assert.doesNotMatch(response.error.message,/module.*not found|cannot find|fetch failed|ECONNREFUSED|ENOENT/i);
  errors.push(response.error);
}
assert.equal(errors[0].message,'Unsupported SDK adapter request');
// Exercise positive resource reads and lazy/native modules without an RPC,
// signer, wallet, simulation, transaction or backend request.
const probe=run(['--input-type=module','-e',`
  import fs from 'node:fs';import path from 'node:path';import {pathToFileURL} from 'node:url';
  import {createRequire} from 'node:module';import {createHash} from 'node:crypto';
  const url=name=>pathToFileURL(path.resolve(name));
  const require=createRequire(url('sdk/package.json'));
  const sdk=await import(url('sdk/dist/protocol/index.js'));
  const governance=await import(url('sdk/node_modules/@amoeba/spread-release-tools/dist/amoebaGovernanceGate.js'));
  const manifests=await Promise.all(['ramx','nandx'].map(product=>sdk.loadCurrentGovernedSkuManifest(product)));
  const native=require('bufferutil');
  const verifier=fs.readFileSync(process.platform==='win32'?'compressed-verifier.exe':'compressed-verifier');
  const receipt=JSON.parse(fs.readFileSync('compressed-verifier.json','utf8'));
  if(createHash('sha256').update(verifier).digest('hex')!==receipt.sha256)throw Error('Verifier digest differs');
  const source=require.resolve('@amoeba/spread-release-tools/compressed-evidence-verifier-source');
  if(!fs.existsSync(source)||typeof native.mask!=='function'||governance.GOVERNANCE_TAIL_LEN!==16)throw Error('Runtime resource missing');
  process.stdout.write(JSON.stringify({manifests:manifests.map(m=>({product:m.product,root:m.requiredSkuRoot.toString('hex'),count:m.requiredSkuCount})),verifierSha256:receipt.sha256}));
`]);
assert.equal(probe.status,0,probe.stderr||probe.stdout);
const resources=JSON.parse(probe.stdout);
const verifier=path.join(stage,process.platform==='win32'?'compressed-verifier.exe':'compressed-verifier');
fs.chmodSync(verifier,0o700);
const rejection=spawnSync(verifier,[],{input:'{}',encoding:'utf8',windowsHide:true,timeout:10000});
assert.equal(rejection.error,undefined);assert.equal(rejection.signal,null);
assert.notEqual(rejection.status,0,'Native verifier must reject a missing proof');
console.log(JSON.stringify({files:manifest.files.length,cases:cases.length,errors,resources,nativeVerifierRejected:true,
  qualification:'offline startup/resources/rejection only; no live business-flow qualification'}));
