import { spawnSync } from 'node:child_process';
import process from 'node:process';

const args = process.argv.slice(2);
const npxCmd = process.platform === 'win32' ? 'npx.cmd' : 'npx';
const res = spawnSync(npxCmd, ['tauri', 'build', ...args], {
  stdio: 'inherit',
});
if (res.status !== 0) {
  process.exit(res.status ?? 1);
}

if (process.platform === 'darwin') {
  const signRes = spawnSync('bash', ['scripts/sign-stable-dr.sh'], {
    stdio: 'inherit',
  });
  if (signRes.status !== 0) {
    process.exit(signRes.status ?? 1);
  }
}
