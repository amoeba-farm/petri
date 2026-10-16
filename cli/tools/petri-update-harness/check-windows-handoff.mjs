import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';
import {execFileSync,spawn} from 'node:child_process';
import assert from 'node:assert/strict';

if(process.platform!=='win32') throw new Error('This check exercises the native Windows process lock.');
const root=path.dirname(fileURLToPath(import.meta.url));
const target=path.join(root,'target');
const oldExecutable=path.join(target,'debug','petri-update-harness.exe');
const newExecutable=path.join(target,'next-version.exe');
const hash=file=>createHash('sha256').update(fs.readFileSync(file)).digest('hex');
assert(fs.existsSync(oldExecutable) && fs.existsSync(newExecutable),'Build the two fixture binaries first.');
const fixture=fs.mkdtempSync(path.join(target,'native-check-'));
// Hosted runners give their shared workspace broad inherited write permissions.
// Model the required private user installation without weakening updater ACLs.
execFileSync('powershell.exe',['-NoProfile','-NonInteractive','-Command',
  '$ErrorActionPreference="Stop"; $fixtureSid=[System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value; $fixtureAcl=New-Object System.Security.AccessControl.DirectorySecurity; $fixtureAcl.SetSecurityDescriptorSddlForm("O:"+$fixtureSid+"D:P(A;OICI;FA;;;"+$fixtureSid+")",[System.Security.AccessControl.AccessControlSections]::Access -bor [System.Security.AccessControl.AccessControlSections]::Owner); [System.IO.Directory]::SetAccessControl($env:PETRI_FIXTURE_ROOT,$fixtureAcl)'
],{windowsHide:true,stdio:'pipe',env:{...process.env,PETRI_FIXTURE_ROOT:fixture}});
const app=path.join(fixture,'Petri'); fs.mkdirSync(app);
const name='.petri-update-0123456789abcdef0123456789abcdef';
const stage=path.join(app,name); fs.mkdirSync(stage);
for(const folder of ['backup','package']) fs.mkdirSync(path.join(stage,folder));
fs.copyFileSync(oldExecutable,path.join(app,'petri.exe'));
fs.copyFileSync(oldExecutable,path.join(stage,'petri-update-helper.exe'));
fs.copyFileSync(oldExecutable,path.join(stage,'backup','petri.exe'));
fs.copyFileSync(newExecutable,path.join(stage,'package','petri.exe'));
fs.writeFileSync(path.join(app,'README.txt'),'old application readme');
fs.writeFileSync(path.join(stage,'backup','README.txt'),'old application readme');
fs.writeFileSync(path.join(stage,'package','README.txt'),'new application readme');
fs.writeFileSync(path.join(app,'user-config.json'),'user-owned configuration sentinel');
const originalHash=hash(oldExecutable);
const files=Object.fromEntries(['README.txt','petri.exe'].map(name=>[name,hash(path.join(stage,'package',name))]));
const originals=Object.fromEntries(['README.txt','petri.exe'].map(name=>[name,hash(path.join(app,name))]));
fs.writeFileSync(path.join(stage,'plan.json'),JSON.stringify({protocol:1,platform:'windows-x64',fromVersion:'0.1.3',toVersion:'0.1.4',parentPid:1,restart:false,createdAt:Math.floor(Date.now()/1000),files,originals}));
fs.writeFileSync(path.join(app,'.petri-update.lock'),name);
const parent=spawn(path.join(app,'petri.exe'),['--fixture-parent',stage],{cwd:app,windowsHide:true,stdio:['ignore','pipe','pipe']});
let output='',errors='',checkedWhileLocked=false;
parent.stdout.on('data',chunk=>{
  output+=chunk;
  if(output.includes('helper-ready-parent-still-running') && !checkedWhileLocked) {
    assert.equal(hash(path.join(app,'petri.exe')),originalHash,'The running executable must remain unchanged.');
    checkedWhileLocked=true;
  }
});
parent.stderr.on('data',chunk=>{errors+=chunk;});
const parentResult=await new Promise((resolve,reject)=>{
  const timer=setTimeout(()=>{parent.kill();reject(new Error('Fixture parent timed out; no real Petri process was touched.'));},45000);
  parent.on('error',error=>{clearTimeout(timer);reject(error);});
  parent.on('close',code=>{clearTimeout(timer);resolve(code);});
});
assert.equal(parentResult,0,errors);
assert(checkedWhileLocked,'The helper handshake was not observed.');
let status;
const deadline=Date.now()+45000;
while(Date.now()<deadline) {
  try { status=JSON.parse(fs.readFileSync(path.join(stage,'status.json'),'utf8')); } catch {}
  if(status?.status==='failed') throw new Error(status.message);
  if(status?.status==='installed' && !fs.existsSync(path.join(app,'.petri-update.lock'))) break;
  await new Promise(resolve=>setTimeout(resolve,100));
}
assert.equal(status?.status,'installed',JSON.stringify(status));
assert.equal(hash(path.join(app,'petri.exe')),hash(newExecutable));
assert.equal(fs.readFileSync(path.join(app,'README.txt'),'utf8'),'new application readme');
assert.equal(fs.readFileSync(path.join(app,'user-config.json'),'utf8'),'user-owned configuration sentinel');
assert.equal(hash(path.join(stage,'backup','petri.exe')),originalHash);
console.log('Native Windows handoff passed: waits for the locked parent, atomically replaces the app, preserves user files and a recovery copy.');
// Only this fresh fixture, inside this harness's ignored target directory.
assert.equal(path.dirname(path.resolve(fixture)),path.resolve(target));
assert(path.basename(fixture).startsWith('native-check-'));
fs.rmSync(fixture,{recursive:true,maxRetries:10,retryDelay:200});
