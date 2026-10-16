import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {ResidentPetri} from './petri-action-bridge.mjs';

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'petri-resident-check-'));
const fixture = path.join(root, 'resident.mjs');
fs.writeFileSync(fixture, `
import readline from 'node:readline'; import fs from 'node:fs';
const mode=process.argv[2], count=process.argv[3];
readline.createInterface({input:process.stdin}).on('line',line=>{
 const request=JSON.parse(line);fs.appendFileSync(count,request.action+'\\n');
 if(mode==='exit')process.exit(1);
 if(mode==='timeout')return;
 if(mode==='invalid')return process.stdout.write('invalid\\n');
 process.stdout.write(JSON.stringify({id:mode==='wrong-id'?request.id+1:request.id,result:{ok:true,pid:process.pid,action:request.action}})+'\\n');
});
`);
try {
  let starts=0;
  const count=path.join(root,'normal.count');
  const normal=new ResidentPetri({command:()=>{starts++;return {bin:process.execPath,prefix:[fixture,'normal',count]};},environment:()=>({}),cwd:root,timeoutMs:2000});
  const responses=await Promise.all([normal.invoke('market.read',{}),normal.invoke('futures.read',{})]);
  assert.equal(starts,1);assert.equal(responses[0].pid,responses[1].pid);
  assert.deepEqual(responses.map(r=>r.action),['market.read','futures.read']);
  await normal.close();
  for(const mode of ['exit','invalid','wrong-id','timeout']) {
    const count=path.join(root,mode+'.count');
    let starts=0;
    const bridge=new ResidentPetri({command:()=>{starts++;return {bin:process.execPath,prefix:[fixture,mode,count]};},environment:()=>({}),cwd:root,timeoutMs:mode==='timeout'?500:2000});
    const result=await bridge.invoke('operations.execute',{operationId:'a'.repeat(64)});
    assert.equal(result.ok,false);assert.equal(result.retryAuthorized,false);
    assert.equal(result.error.category,'pending');assert.equal(result.operationId,'a'.repeat(64));
    assert.equal(starts,1);assert.deepEqual(fs.readFileSync(count,'utf8').trim().split('\n'),['operations.execute']);
    await bridge.close();
  }
  console.log('Resident MCP transport: process reuse, ordered calls, process loss, timeout, invalid output and response identity; no execution replay.');
} finally {
  const relative=path.relative(os.tmpdir(),root);
  assert(!relative.startsWith('..')&&!path.isAbsolute(relative));
  fs.rmSync(root,{recursive:true,force:true});
}
