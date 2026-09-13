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
  fs.mkdirSync(path.join(app,'Contents/MacOS'),{recursive:true});
  run('xcrun',['clang','-Os','-Wall','-Wextra','-Werror','-mmacosx-version-min=14.0','scripts/petri-macos-launcher.c','-o',path.join(app,'Contents/MacOS/Petri')]);
  write(path.join(app,'Contents/Info.plist'),`<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleExecutable</key><string>Petri</string><key>CFBundleIdentifier</key><string>farm.amoeba.petri.preview</string><key>CFBundleName</key><string>Petri</string><key>CFBundlePackageType</key><string>APPL</string><key>CFBundleIconFile</key><string>Petri.icns</string><key>CFBundleVersion</key><string>${version}</string><key>CFBundleShortVersionString</key><string>${version}</string><key>LSMinimumSystemVersion</key><string>14.0</string></dict></plist>\n`);
}
const exe=platform==='windows'?'petri.exe':'petri';
const updateInfo=JSON.parse(execFileSync(path.join(root,'target/release',exe),['--json','update','info'],{encoding:'utf8',windowsHide:true}));
const updatePlatform=platform==='windows'?'windows-x64':`macos-${process.arch==='arm64'?'arm64':'x86_64'}`;
if(updateInfo.protocol!==1 || updateInfo.channel!=='preview' || updateInfo.version!==version || updateInfo.platform!==updatePlatform) {
  throw new Error('Preview packaging requires a matching PETRI_UPDATE_CHANNEL=preview binary');
}
write(path.join(binDir,'petri-update.json'),JSON.stringify(updateInfo)+'\n');
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
write(path.join(stage,'README.txt'),`Petri ${version} — Devnet preview\n\nThis preview is not publisher-signed or notarized. Verify the release SHA-256 before opening it.\nWindows: install Microsoft's Visual C++ v14 x64 runtime if needed (https://aka.ms/vc14/vc_redist.x64.exe), then double-click Petri.cmd, or run powershell -ExecutionPolicy Bypass -File .\\install-preview.ps1\nmacOS: open Petri.app (opens Terminal), or run bash ./install-preview.sh\nmacOS may require Privacy & Security > Open Anyway for this specific app. Do not disable system security globally.\nThe installer adds the petri terminal command; restart your terminal afterward.\nUpdates: petri update check, then petri update. Confirm the displayed preview release and close other Petri windows. Use petri update recover to restore the saved previous app files. Wallets and settings are not updated. Signed installers remain separate.\nSource and install instructions: https://github.com/amoeba-farm/petri\n`);
if(platform==='macos') {
  run('codesign',['--force','--deep','--sign','-',path.join(stage,'Petri.app')]);
  run('codesign',['--verify','--deep','--strict',path.join(stage,'Petri.app')]);
}
const walk=(dir,prefix='')=>fs.readdirSync(dir,{withFileTypes:true}).flatMap(e=>e.isDirectory()?walk(path.join(dir,e.name),prefix+e.name+'/'):[prefix+e.name]);
const digest=file=>createHash('sha256').update(fs.readFileSync(file)).digest('hex');
write(path.join(stage,'SHA256SUMS'),walk(stage).sort().map(f=>`${digest(path.join(stage,f))}  ${f}\n`).join(''));
const archive=path.join(root,'dist',stem+(platform==='linux'?'.tar.gz':'.zip'));
if(platform==='macos') {
  // All signatures are ordinary files or embedded Mach-O data, not xattrs.
  // Do not add AppleDouble metadata entries outside the updater's file manifest.
  run('ditto',['-c','-k','--norsrc','--noextattr','--noacl','--keepParent',stage,archive]);
  const roundtrip=fs.mkdtempSync(path.join(root,'dist','petri-mac-roundtrip-'));
  try {
    run('ditto',['-x','-k',archive,roundtrip]);
    run('codesign',['--verify','--deep','--strict',path.join(roundtrip,stem,'Petri.app')]);
  } finally {
    if(path.dirname(roundtrip)!==path.join(root,'dist') || !path.basename(roundtrip).startsWith('petri-mac-roundtrip-')) throw new Error('Invalid package verification directory');
    fs.rmSync(roundtrip,{recursive:true});
  }
}
else if(platform==='windows') {
  run('powershell',['-NoProfile','-Command',`Compress-Archive -LiteralPath '${stage.replaceAll("'","''")}' -DestinationPath '${archive.replaceAll("'","''")}'`]);
} else run('tar',['-czf',archive,'-C',path.dirname(stage),stem]);
write(archive+'.sha256',`${digest(archive)}  ${path.basename(archive)}\n`);
console.log('Preview archive: '+archive);
