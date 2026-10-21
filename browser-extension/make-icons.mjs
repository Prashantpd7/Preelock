// The extension icons are generated from the checked-in supplied logo asset.
// Platform-specific resizing is performed when the assets are updated.
import { existsSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const dir = dirname(fileURLToPath(import.meta.url));
for (const size of [16, 32, 48, 128]) {
  const file = join(dir, `icon${size}.png`);
  if (!existsSync(file)) throw new Error(`Missing supplied logo icon: ${file}`);
  console.log(`using ${file}`);
}
