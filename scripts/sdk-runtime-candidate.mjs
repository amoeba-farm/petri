// Deliberately opt-in. This static module projection is not evidence that
// filesystem resources, computed imports or all rejection paths are qualified.
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';

export async function bundleWorkerSdk(root, stage, sdk) {
  const source = ['sdk-worker.mjs', 'sdk-oracle.mjs', 'sdk-carry.mjs']
    .map(name => fs.readFileSync(path.join(root, 'scripts', name), 'utf8')).join('\n');
  if (/sdk\s*\[/.test(source)) throw new Error('Computed SDK access needs an explicit bundling review');
  const names = [...new Set([...source.matchAll(/\bsdk\.([A-Za-z_$][\w$]*)/g)].map(match => match[1]))].sort();
  if (!names.length) throw new Error('No static SDK entrypoints found');
  const esbuild = createRequire(path.join(sdk,'package.json'))('esbuild');
  const output = 'sdk/dist/protocol/index.js';
  // Companions also import internal SDK modules directly. Keep each as an
  // entrypoint, including its transitive graph, instead of dropping it merely
  // because it was included in the protocol entrypoint's module graph.
  const direct = new Set([...source.matchAll(/['"]\.\/(sdk\/dist\/[^'"]+\.js)['"]/g)].map(match => match[1]));
  const entry = path.join(stage,'petri-sdk-entry.mjs');
  fs.writeFileSync(entry,`export { ${names.join(', ')} } from './sdk/dist/protocol/index.js';`);
  const entryPoints = Object.fromEntries([...direct].map(name => [
    name.slice(0,-3), name === output ? entry : path.join(stage,name),
  ]));
  const result = await esbuild.build({
    absWorkingDir:stage,
    entryPoints, outdir:stage, write:false, allowOverwrite:true, bundle:true, splitting:true,
    chunkNames:'sdk/dist/petri-chunks/[name]-[hash]', platform:'node', format:'esm', target:'node22',
    packages:'external', mainFields:['main'], conditions:['node'], keepNames:true,
    treeShaking:true, minify:false, sourcemap:false, metafile:true, legalComments:'inline',
  });
  const outputs = new Set(result.outputFiles.map(file => path.relative(stage,file.path).split(path.sep).join('/')));
  const included = Object.keys(result.metafile.inputs).map(name => name.replaceAll('\\', '/'))
    .filter(name => name.startsWith('sdk/dist/') && name.endsWith('.js') && !outputs.has(name));
  if (!outputs.has(output)) throw new Error('Missing candidate SDK bundle');
  for (const generated of result.outputFiles) {
    fs.mkdirSync(path.dirname(generated.path),{recursive:true});
    fs.writeFileSync(generated.path,generated.contents);
  }
  fs.writeFileSync(path.join(stage, 'worker-bundle-metafile.json'), JSON.stringify(result.metafile, null, 2));
  return included;
}
