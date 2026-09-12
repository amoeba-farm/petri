// One offline launch smoke: verify packaged bytes and run the bundled Node worker.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {inflateSync} from 'node:zlib';
import {spawnSync} from 'node:child_process';
import {readRuntime,sha256} from './sdk-runtime-archive.mjs';
const {bundle,manifest}=readRuntime('target/petri-sdk-runtime.bin');
const start=8+Number(bundle.readBigUInt64LE());
const stage=fs.mkdtempSync(path.join(os.tmpdir(),'petri-runtime-smoke-'));
for(const entry of manifest.files){
  if(entry.path.includes('\\')||entry.path.split('/').some(p=>!p||p==='.'||p==='..')||entry.path.includes(':'))throw new Error('Unsafe runtime path');
  const bytes=inflateSync(bundle.subarray(start+entry.offset,start+entry.offset+entry.length),{maxOutputLength:128*1024*1024});
  if(bytes.length!==entry.bytes||sha256(bytes)!==entry.sha256)throw new Error('Runtime checksum mismatch: '+entry.path);
  const file=path.join(stage,entry.path);fs.mkdirSync(path.dirname(file),{recursive:true});fs.writeFileSync(file,bytes);
}
const node=path.join(stage,process.platform==='win32'?'node.exe':'node');
fs.chmodSync(node,0o700);
const result=spawnSync(node,[path.join(stage,'worker.mjs')],{input:'{"schemaVersion":0}',encoding:'utf8',windowsHide:true,timeout:30000});
let response;try{response=JSON.parse(result.stdout)}catch{throw new Error('Packaged SDK worker failed to start: '+(result.stderr||result.error));}
if(result.status!==1||response.ok!==false||response.error?.message!=='Unsupported SDK adapter request')throw new Error('Unexpected offline SDK worker response');
console.log(`Verified ${manifest.files.length} packaged file checksums and bundled Node/SDK startup; no network, wallet or transaction request.`);
