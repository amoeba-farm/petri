// Static file tracing only. Retained SDK modules keep their original bytes,
// module identities and paths; no bundling or runtime dependency installation.
import fs from 'node:fs';
import path from 'node:path';
import { sha256 } from './sdk-runtime-archive.mjs';

export const RUNTIME_LAYOUT = 'worker-file-closure-v2';
export const WORKER_FILES = ['worker.mjs', 'sdk-oracle.mjs', 'sdk-carry.mjs'];
const legalFile = /(?:^|\/)(?:[^/]*LICENSE[^/]*|NOTICE[^/]*|COPYING[^/]*)$/i;
const installationArchive = /^sdk\/vendor\/[^/]+\.tgz$/;
const normalized = name => name.split(path.sep).join('/');
const safePath = name => !/[\\:\0]/.test(name) && !name.split('/').some(part => !part || part === '.' || part === '..');

export function runtimeInputFiles(stage) {
  const files = [];
  function walk(directory) {
    for (const entry of fs.readdirSync(path.join(stage, directory), { withFileTypes: true })) {
      if (entry.name === '.bin' || entry.name.endsWith('.map') || entry.name.endsWith('.d.ts')) continue;
      const name = path.posix.join(directory, entry.name);
      if (entry.isSymbolicLink()) throw new Error(`Unexpected SDK dependency symlink: ${name}`);
      if (entry.isDirectory()) walk(name);
      else if (entry.isFile()) files.push(name);
      else throw new Error(`Unsupported runtime file: ${name}`);
    }
  }
  for (const directory of ['sdk/dist', 'sdk/node_modules', 'sdk/release', 'sdk/vendor']) walk(directory);
  for (const entry of fs.readdirSync(path.join(stage, 'sdk'), { withFileTypes: true })) {
    if (legalFile.test(entry.name) && entry.isFile()) files.push(`sdk/${entry.name}`);
  }
  files.push('sdk/package.json', 'sdk/package-lock.json');
  return files.sort();
}

function resourceReason(name) {
  if (legalFile.test(name)) return 'License/notice corpus, including removed dependency attribution';
  if (name === 'sdk/package.json' || name === 'sdk/package-lock.json') return 'Immutable SDK installation provenance';
  if (/^sdk\/(?:release|vendor)\/.*\.json$/.test(name)) return 'SDK release identity, governed SKU data and vendor provenance';
  if (name.startsWith('sdk/node_modules/@amoeba/spread-release-tools/tools/current-compressed-evidence-verifier/')) {
    return 'Native verifier source-locator contract and provenance';
  }
  return null;
}

function packageDirectory(name) {
  const marker = name.lastIndexOf('/node_modules/');
  if (marker < 0) return null;
  const start = marker + '/node_modules/'.length;
  const parts = name.slice(start).split('/');
  if (parts.length < (parts[0].startsWith('@') ? 3 : 2)) return null;
  return name.slice(0, start) + parts.slice(0, parts[0].startsWith('@') ? 2 : 1).join('/');
}

export async function selectRuntimeFiles(stage, files, { nodeFileTrace, resolve }) {
  stage = path.resolve(stage);
  files ??= runtimeInputFiles(stage);
  const roots = WORKER_FILES.map(name => path.join(stage, name));
  const insideStage = name => {
    const relative = path.relative(stage, path.resolve(name));
    return relative === '' || (!path.isAbsolute(relative) && relative !== '..' && !relative.startsWith(`..${path.sep}`));
  };
  // Never let an ancestor checkout's node_modules make a clean build appear
  // complete. The three workers bind createRequire to sdk/package.json.
  const isolatedRead = async (name, operation) => {
    if (!insideStage(name)) return null;
    try { return await operation(name); }
    catch (error) {
      if (['ENOENT', 'ENOTDIR', 'EISDIR', 'EINVAL', 'UNKNOWN'].includes(error.code)) return null;
      throw error;
    }
  };
  const optionalImports = [];
  const traced = await nodeFileTrace(roots, {
    base: stage, processCwd: stage, exportsOnly: true, conditions: ['node'],
    // Trace both current Node module-sync exports and their fallback branches.
    moduleSyncCatchall: true,
    stat: name => isolatedRead(name, fs.promises.stat),
    readFile: name => isolatedRead(name, file => fs.promises.readFile(file, 'utf8')),
    readlink: name => isolatedRead(name, fs.promises.readlink),
    resolve: async (id, parent, job, isCjs) => {
      const workerRequire = roots.includes(path.resolve(parent)) && ['@solana/web3.js', '@lightprotocol/stateless.js'].includes(id);
      const sdkParent = workerRequire ? path.join(stage, 'sdk/package.json') : parent;
      try { return await resolve(id, sdkParent, job, workerRequire || isCjs); }
      catch (error) {
        // node-fetch 2.7.0 catches this absent optional peer. It is needed
        // only by textConverted(), not its JSON/UTF-8 paths. Bind the exception
        // to the exact reviewed bytes; every other unresolved import fails.
        const relativeParent = normalized(path.relative(stage, parent));
        if (error.code !== 'MODULE_NOT_FOUND' || id !== 'encoding' ||
            relativeParent !== 'sdk/node_modules/node-fetch/lib/index.js' ||
            sha256(fs.readFileSync(parent)) !== '79541f01338e70f4ff6f8a1d12f5c2914d4c6ab2ba96b7ddf48d2add01c13813') throw error;
        optionalImports.push({ parent: relativeParent, id, reason: 'Pinned node-fetch catches its absent optional encoding peer' });
        return [];
      }
    },
  });
  if (traced.warnings.size) {
    throw new Error(`Worker dependency trace needs review:\n${[...traced.warnings].map(w => w.message).join('\n')}`);
  }
  const tracedFiles = new Set([...traced.fileList].map(normalized));
  const rootFiles = [...WORKER_FILES, 'compressed-verifier.json',
    process.platform === 'win32' ? 'compressed-verifier.exe' : 'compressed-verifier'];
  const available = new Set([...files, ...rootFiles]);
  for (const name of tracedFiles) {
    if (!safePath(name) || !available.has(name) || installationArchive.test(name)) {
      throw new Error(`Worker needs an unreviewed or installation-only input: ${name}`);
    }
    if (!fs.lstatSync(path.join(stage, name)).isFile()) throw new Error(`Worker input is not a regular file: ${name}`);
  }
  const included = [], omitted = [], resources = [];
  for (const name of files) {
    const resource = resourceReason(name);
    if (tracedFiles.has(name) || resource) {
      included.push(name);
      if (resource) resources.push({ path: name, reason: resource });
    } else {
      const bytes = fs.readFileSync(path.join(stage, name));
      omitted.push({ path: name, bytes: bytes.length, sha256: sha256(bytes), reason: installationArchive.test(name)
        ? 'Installation-only vendor tarball; extracted code and provenance retained'
        : 'Not reachable from worker/Oracle/carry entrypoints and not a retained resource' });
    }
  }
  const retainedPackages = new Set(included.filter(name => !legalFile.test(name)).map(packageDirectory).filter(Boolean));
  const packages = [...retainedPackages].sort().map(directory => {
    const manifest = JSON.parse(fs.readFileSync(path.join(stage, directory, 'package.json'), 'utf8'));
    return { path: directory, name: manifest.name, version: manifest.version };
  });
  return { included, omitted, dependencyClosure: {
    layout: RUNTIME_LAYOUT, tracer: '@vercel/nft@1.11.0', roots: WORKER_FILES,
    platform: process.platform, arch: process.arch, nodeVersion: process.version,
    packages, resources, optionalImports, tracedFiles: [...tracedFiles].sort(), warnings: [],
    reasons: [...traced.reasons].filter(([name]) => tracedFiles.has(normalized(name))).map(([name, reason]) => ({
      path: normalized(name), type: reason.type, parents: [...reason.parents].map(normalized).sort(),
    })).sort((a, b) => a.path.localeCompare(b.path)),
  } };
}
