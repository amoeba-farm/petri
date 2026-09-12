// Explicit unsigned preview packaging. The signed installer/updater stays separate.
import fs from 'node:fs';
import path from 'node:path';
import {execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
const root=process.cwd();
const version=fs.readFileSync('Cargo.toml','utf8').match(/^version = "([0-9]+\.[0-9]+\.[0-9]+)"/m)?.[1];
if(!version) throw new Error('Missing package version');
const platform=process.platform==='win32'?'windows':process.platform==='darwin'?'macos':'linux';
const arch=process.arch==='x64'?(platform==='windows'?'x64':'x86_64'):process.arch;
const stem=`Petri-${platform}-${arch}`;
const stage=path.join(root,'dist',stem);
if(fs.existsSync(stage)) throw new Error('Refusing to overwrite package staging directory');
fs.mkdirSync(stage,{recursive:true});
const run=(cmd,args)=>execFileSync(cmd,args,{cwd:root,stdio:'inherit',windowsHide:true});
const write=(file,text,mode=0o644)=>{fs.mkdirSync(path.dirname(file),{recursive:true});fs.writeFileSync(file,text,{mode});};
let binDir=stage;
if(platform==='macos') {
  const app=path.join(stage,'Petri.app');
  binDir=path.join(app,'Contents/Resources'); fs.mkdirSync(binDir,{recursive:true});
  fs.copyFileSync('assets/Petri.icns',path.join(binDir,'Petri.icns'));
  write(path.join(binDir,'petri.command'),'#!/bin/bash\nset -euo pipefail\nexec "$(cd "$(dirname "$0")" && pwd)/petri" tui\n',0o755);
  write(path.join(app,'Contents/MacOS/Petri'),'#!/bin/bash\nset -euo pipefail\n/usr/bin/open -a Terminal "$(cd "$(dirname "$0")/.." && pwd)/Resources/petri.command"\n',0o755);
  write(path.join(app,'Contents/Info.plist'),`<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleExecutable</key><string>Petri</string><key>CFBundleIdentifier</key><string>farm.amoeba.petri.preview</string><key>CFBundleName</key><string>Petri</string><key>CFBundlePackageType</key><string>APPL</string><key>CFBundleIconFile</key><string>Petri.icns</string><key>CFBundleVersion</key><string>${version}</string><key>CFBundleShortVersionString</key><string>${version}</string><key>LSMinimumSystemVersion</key><string>14.0</string></dict></plist>\n`);
}
const exe=platform==='windows'?'petri.exe':'petri';
fs.copyFileSync(path.join('target/release',exe),path.join(binDir,exe));
fs.chmodSync(path.join(binDir,exe),0o755);
for(const file of ['LICENSE','THIRD_PARTY_NOTICES.md','THIRD_PARTY_LICENSES.md']) fs.copyFileSync(file,path.join(binDir,file));
if(platform==='windows') {
  fs.copyFileSync('assets/Petri.ico',path.join(stage,'Petri.ico'));
  write(path.join(stage,'Petri.cmd'),'@echo off\r\n"%~dp0petri.exe" tui\r\nif errorlevel 1 pause\r\n');
}
const installer=platform==='windows'?'install-preview.ps1':'install-preview.sh';
fs.copyFileSync(path.join('scripts',installer),path.join(stage,installer));
fs.chmodSync(path.join(stage,installer),0o755);
write(path.join(stage,'README.txt'),`Petri ${version} — Devnet preview\n\nThis preview is not publisher-signed or notarized. Verify the release SHA-256 before opening it.\nWindows: double-click Petri.cmd, or run powershell -ExecutionPolicy Bypass -File .\\install-preview.ps1\nmacOS: open Petri.app (opens Terminal), or run bash ./install-preview.sh\nmacOS may require Privacy & Security > Open Anyway for this specific app. Do not disable system security globally.\nLinux: run bash ./install-preview.sh\nThe installer adds the petri terminal command; restart your terminal afterward.\nUpdates: rerun the preview installer. The signed automatic updater does not accept unsigned preview packages.\nSource and install instructions: https://github.com/amoeba-farm/petri\n`);
if(platform==='macos') {
  run('codesign',['--force','--deep','--sign','-',path.join(stage,'Petri.app')]);
  run('codesign',['--verify','--deep','--strict',path.join(stage,'Petri.app')]);
}
const walk=(dir,prefix='')=>fs.readdirSync(dir,{withFileTypes:true}).flatMap(e=>e.isDirectory()?walk(path.join(dir,e.name),prefix+e.name+'/'):[prefix+e.name]);
const digest=file=>createHash('sha256').update(fs.readFileSync(file)).digest('hex');
write(path.join(stage,'SHA256SUMS'),walk(stage).sort().map(f=>`${digest(path.join(stage,f))}  ${f}\n`).join(''));
const archive=path.join(root,'dist',stem+(platform==='linux'?'.tar.gz':'.zip'));
if(platform==='macos') run('ditto',['-c','-k','--keepParent',stage,archive]);
else if(platform==='windows') {
  run('powershell',['-NoProfile','-Command',`Compress-Archive -LiteralPath '${stage.replaceAll("'","''")}' -DestinationPath '${archive.replaceAll("'","''")}'`]);
} else run('tar',['-czf',archive,'-C',path.dirname(stage),stem]);
write(archive+'.sha256',`${digest(archive)}  ${path.basename(archive)}\n`);
console.log('Preview archive: '+archive);
