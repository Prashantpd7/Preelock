import fs from 'node:fs';
import process from 'node:process';

const read = (file) => JSON.parse(fs.readFileSync(file, 'utf8'));
const version = read('package.json').version;
const cargo = fs.readFileSync('src-tauri/Cargo.toml', 'utf8');
const lock = fs.readFileSync('src-tauri/Cargo.lock', 'utf8');
const versions = [
  read('package-lock.json').version,
  read('package-lock.json').packages[''].version,
  read('src-tauri/tauri.conf.json').version,
  cargo.match(/^version = "([^"]+)"/m)?.[1],
  lock.match(/name = "vystriq"\nversion = "([^"]+)"/)?.[1],
];
if (!/^\d+\.\d+\.\d+$/.test(version) || versions.some((v) => v !== version)) {
  throw new Error('App and lockfile versions must match before release');
}
process.stdout.write(`version=${version}\n`);
