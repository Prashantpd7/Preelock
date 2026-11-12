import { Buffer } from 'node:buffer';
import fs from 'node:fs';
import path from 'node:path';
import process from 'node:process';
import zlib from 'node:zlib';

const root = process.cwd();
const pkg = JSON.parse(
  fs.readFileSync(path.join(root, 'package.json'), 'utf8'),
);
const tauriConf = JSON.parse(
  fs.readFileSync(path.join(root, 'src-tauri/tauri.conf.json'), 'utf8'),
);

const resources = Object.entries(tauriConf.bundle?.resources ?? {});
const extensionFiles = resources
  .filter(([, dest]) => dest.startsWith('browser-extension/'))
  .map(([src, dest]) => ({
    srcPath: path.resolve(root, 'src-tauri', src),
    zipName: dest.slice('browser-extension/'.length),
  }));

if (extensionFiles.length === 0) {
  throw new Error(
    'No browser-extension resources declared in tauri.conf.json.',
  );
}

const forbiddenNames = [/\.test\.js$/i, /^make-icons\.mjs$/i, /\.DS_Store$/];
const entries = extensionFiles.map(({ srcPath, zipName }) => {
  if (forbiddenNames.some((re) => re.test(zipName))) {
    throw new Error(
      `Forbidden non-runtime file in extension bundle: ${zipName}`,
    );
  }
  let data = fs.readFileSync(srcPath);
  if (zipName === 'manifest.json') {
    const manifest = JSON.parse(data.toString('utf8'));
    if (manifest.manifest_version !== 3) {
      throw new Error(
        `Expected MV3 manifest, found v${manifest.manifest_version}.`,
      );
    }
    if (manifest.version !== pkg.version) {
      throw new Error(
        `Version mismatch: manifest.json (${manifest.version}) != package.json (${pkg.version}).`,
      );
    }
    if ('key' in manifest || 'update_url' in manifest) {
      throw new Error(
        'Production CWS manifest must not set key or update_url.',
      );
    }
    manifest.permissions = (manifest.permissions ?? []).filter(
      (p) => p !== 'declarativeNetRequestFeedback',
    );
    data = Buffer.from(`${JSON.stringify(manifest, null, 2)}\n`, 'utf8');
  }
  return { name: zipName.replace(/\\/g, '/'), data };
});

function crc32(buf) {
  let crc = 0xffffffff;
  for (let i = 0; i < buf.length; i++) {
    crc ^= buf[i];
    for (let j = 0; j < 8; j++) {
      crc = (crc >>> 1) ^ (crc & 1 ? 0xedb88320 : 0);
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function buildZip(files) {
  const localChunks = [];
  const centralChunks = [];
  let offset = 0;

  for (const file of files) {
    const nameBuf = Buffer.from(file.name, 'utf8');
    const crc = crc32(file.data);
    const compressed = zlib.deflateRawSync(file.data);
    const useDeflate = compressed.length < file.data.length;
    const payload = useDeflate ? compressed : file.data;
    const method = useDeflate ? 8 : 0;

    const localHeader = Buffer.alloc(30);
    localHeader.writeUInt32LE(0x04034b50, 0);
    localHeader.writeUInt16LE(20, 4);
    localHeader.writeUInt16LE(0x0800, 6);
    localHeader.writeUInt16LE(method, 8);
    localHeader.writeUInt16LE(0, 10);
    localHeader.writeUInt16LE(0x5421, 12);
    localHeader.writeUInt32LE(crc, 14);
    localHeader.writeUInt32LE(payload.length, 18);
    localHeader.writeUInt32LE(file.data.length, 22);
    localHeader.writeUInt16LE(nameBuf.length, 26);
    localHeader.writeUInt16LE(0, 28);

    localChunks.push(localHeader, nameBuf, payload);

    const centralHeader = Buffer.alloc(46);
    centralHeader.writeUInt32LE(0x02014b50, 0);
    centralHeader.writeUInt16LE(20, 4);
    centralHeader.writeUInt16LE(20, 6);
    centralHeader.writeUInt16LE(0x0800, 8);
    centralHeader.writeUInt16LE(method, 10);
    centralHeader.writeUInt16LE(0, 12);
    centralHeader.writeUInt16LE(0x5421, 14);
    centralHeader.writeUInt32LE(crc, 16);
    centralHeader.writeUInt32LE(payload.length, 20);
    centralHeader.writeUInt32LE(file.data.length, 24);
    centralHeader.writeUInt16LE(nameBuf.length, 28);
    centralHeader.writeUInt16LE(0, 30);
    centralHeader.writeUInt16LE(0, 32);
    centralHeader.writeUInt16LE(0, 34);
    centralHeader.writeUInt16LE(0, 36);
    centralHeader.writeUInt32LE(0, 38);
    centralHeader.writeUInt32LE(offset, 42);

    centralChunks.push(centralHeader, nameBuf);
    offset += localHeader.length + nameBuf.length + payload.length;
  }

  const centralSize = centralChunks.reduce((sum, b) => sum + b.length, 0);
  const eocd = Buffer.alloc(22);
  eocd.writeUInt32LE(0x06054b50, 0);
  eocd.writeUInt16LE(0, 4);
  eocd.writeUInt16LE(0, 6);
  eocd.writeUInt16LE(files.length, 8);
  eocd.writeUInt16LE(files.length, 10);
  eocd.writeUInt32LE(centralSize, 12);
  eocd.writeUInt32LE(offset, 16);
  eocd.writeUInt16LE(0, 20);

  return Buffer.concat([...localChunks, ...centralChunks, eocd]);
}

const outDir = path.join(root, 'dist');
fs.mkdirSync(outDir, { recursive: true });
const outPath = path.join(
  outDir,
  `preelock-focus-extension-v${pkg.version}-chrome-web-store.zip`,
);
fs.writeFileSync(outPath, buildZip(entries));
process.stdout.write(
  `Built ${path.relative(root, outPath)} (${entries.length} files, version ${pkg.version})\n`,
);
