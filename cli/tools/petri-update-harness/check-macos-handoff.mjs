import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';
import {execFileSync,spawn} from 'node:child_process';
import assert from 'node:assert/strict';

if(process.platform!=='darwin') throw new Error('Native macOS qualification only.');
const root=path.dirname(fileURLToPath(import.meta.url));
const target=path.join(root,'target');
const fixture=fs.mkdtempSync(path.join(target,'mac-native-check-')); fs.chmodSync(fixture,0o700);
const app=path.join(fixture,'Petri.app');
const storage=path.join(fixture,'.petri-app-updates'); fs.mkdirSync(storage,{mode:0o700});
const name='.petri-update-0123456789abcdef0123456789abcdef';
const stage=path.join(storage,name); fs.mkdirSync(stage,{mode:0o700});
const packageApp=path.join(stage,'package','Petri.app');
const oldBinary=path.join(target,'debug','petri-update-harness');
const newBinary=path.join(target,'next-version');
const hash=file=>createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const run=(command,args)=>execFileSync(command,args,{stdio:'pipe'});
function makeApp(destination,binary,version) {
  fs.mkdirSync(path.join(destination,'Contents/MacOS'),{recursive:true});
  fs.mkdirSync(path.join(destination,'Contents/Resources'),{recursive:true});
  run('xcrun',['clang','-Os','-Wall','-Wextra','-Werror',path.resolve(root,'../../scripts/petri-macos-launcher.c'),'-o',path.join(destination,'Contents/MacOS/Petri')]);
  fs.copyFileSync(binary,path.join(destination,'Contents/Resources/petri'));
  fs.chmodSync(path.join(destination,'Contents/Resources/petri'),0o755);
  fs.writeFileSync(path.join(destination,'Contents/Resources/petri.command'),'#!/bin/bash\nexit 0\n',{mode:0o755});
  fs.writeFileSync(path.join(destination,'Contents/Info.plist'),`<?xml version="1.0"?><plist version="1.0"><dict><key>CFBundleExecutable</key><string>Petri</string><key>CFBundleIdentifier</key><string>farm.amoeba.petri.fixture</string><key>CFBundlePackageType</key><string>APPL</string><key>CFBundleVersion</key><string>${version}</string></dict></plist>`);
  run('/usr/bin/codesign',['--force','--deep','--sign','-',destination]);
  run('/usr/bin/codesign',['--verify','--deep','--strict',destination]);
}
makeApp(app,oldBinary,'0.1.3'); makeApp(packageApp,newBinary,'0.1.4');
const walk=(dir,prefix='')=>fs.readdirSync(dir,{withFileTypes:true}).flatMap(e=>e.isDirectory()?walk(path.join(dir,e.name),prefix+e.name+'/'):[prefix+e.name]);
const files=Object.fromEntries(walk(packageApp).map(file=>[file,hash(path.join(packageApp,file))]));
const originals=Object.fromEntries(Object.keys(files).map(file=>[file,hash(path.join(app,file))]));
for(const file of Object.keys(files)) {
  const backup=path.join(stage,'backup',file); fs.mkdirSync(path.dirname(backup),{recursive:true});
  fs.copyFileSync(path.join(app,file),backup);
}
fs.copyFileSync(path.join(app,'Contents/Resources/petri'),path.join(stage,'petri-update-helper'));
fs.chmodSync(path.join(stage,'petri-update-helper'),0o755);
fs.writeFileSync(path.join(stage,'plan.json'),JSON.stringify({protocol:1,platform:process.arch==='arm64'?'macos-arm64':'macos-x86_64',appName:'Petri.app',fromVersion:'0.1.3',toVersion:'0.1.4',parentPid:1,restart:false,createdAt:Math.floor(Date.now()/1000),files,originals}));
fs.writeFileSync(path.join(storage,'.petri-update.lock'),name);
fs.writeFileSync(path.join(fixture,'user-config.json'),'untouched fixture configuration');
const parent=spawn(path.join(app,'Contents/Resources/petri'),['--fixture-parent',stage],{stdio:['ignore','pipe','pipe']});
let output='',errors='',observed=false;
parent.stdout.on('data',data=>{
  output+=data;
  if(output.includes('helper-ready-parent-still-running') && !observed) {
    assert.equal(hash(path.join(app,'Contents/Resources/petri')),originals['Contents/Resources/petri']); observed=true;
  }
});
parent.stderr.on('data',data=>{errors+=data;});
const exit=await new Promise((resolve,reject)=>{
  const timer=setTimeout(()=>{parent.kill();reject(new Error('Fixture timed out.'));},45000);
  parent.on('error',error=>{clearTimeout(timer);reject(error);});
  parent.on('close',code=>{clearTimeout(timer);resolve(code);});
});
assert.equal(exit,0,errors); assert(observed);
const status=JSON.parse(fs.readFileSync(path.join(stage,'status.json'),'utf8'));
assert.equal(status.status,'installed',status.message);
assert(!fs.existsSync(path.join(storage,'.petri-update.lock')));
for(const [file,digest] of Object.entries(files)) assert.equal(hash(path.join(app,file)),digest);
assert.equal(fs.readFileSync(path.join(fixture,'user-config.json'),'utf8'),'untouched fixture configuration');
run('/usr/bin/codesign',['--verify','--deep','--strict',app]);
assert.equal(run(path.join(app,'Contents/Resources/petri'),['--version']).toString().trim(),'petri 0.1.4');
assert(!fs.readdirSync(app).some(file=>file.startsWith('.petri-')));
console.log('Native Mac update passed: parent exit, external staging, exact replacement, valid app signature, startup and untouched configuration.');
assert.equal(path.dirname(path.resolve(fixture)),path.resolve(target));
assert(path.basename(fixture).startsWith('mac-native-check-'));
fs.rmSync(fixture,{recursive:true});
