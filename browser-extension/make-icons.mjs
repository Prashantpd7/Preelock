// Generates the extension toolbar icons from the VYSTRIQ logo geometry
// (same artwork as src-tauri/icons/app.svg). Pure Node.js, no dependencies.
// Run once: node browser-extension/make-icons.mjs
import { writeFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { deflateSync } from 'node:zlib';

const dir = dirname(fileURLToPath(import.meta.url));
const BG = [0x14, 0x17, 0x15]; // #141715
const FG = [0xc3, 0xed, 0x9b]; // #c3ed9b
const RADIUS = 110;
// V polygon in the 512 viewBox of app.svg
const V = [
  [112, 130],
  [194, 130],
  [256, 301],
  [318, 130],
  [400, 130],
  [286, 394],
  [226, 394],
];

function pointInPolygon(x, y, poly) {
  let inside = false;
  for (let i = 0, j = poly.length - 1; i < poly.length; j = i++) {
    const [xi, yi] = poly[i];
    const [xj, yj] = poly[j];
    if (yi > y !== yj > y && x < ((xj - xi) * (y - yi)) / (yj - yi) + xi)
      inside = !inside;
  }
  return inside;
}

function inRoundedRect(x, y, s, r) {
  const cx = Math.min(Math.max(x, r), s - r);
  const cy = Math.min(Math.max(y, r), s - r);
  const dx = x - cx;
  const dy = y - cy;
  return (
    dx * dx + dy * dy <= r * r ||
    (x >= r && x <= s - r) ||
    (y >= r && y <= s - r)
  );
}

// Simple CRC32 for PNG chunks.
const crcTable = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();
function crc32(bytes) {
  let c = 0xffffffff;
  for (const b of bytes) c = crcTable[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, 'ascii'), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}

function encodePng(rgba, w, h) {
  const rows = [];
  for (let y = 0; y < h; y++) {
    rows.push(0x00);
    for (let x = 0; x < w; x++) {
      const i = (y * w + x) * 4;
      rows.push(rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]);
    }
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(h, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // color type: RGBA
  const png = Buffer.concat([
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    chunk('IHDR', ihdr),
    chunk('IDAT', deflateSync(Buffer.from(rows))),
    chunk('IEND', Buffer.alloc(0)),
  ]);
  return png;
}

function renderIcon(size) {
  // 4x supersampling for smooth edges.
  const SS = 4;
  const big = size * SS;
  const k = big / 512;
  const poly = V.map(([x, y]) => [x * k, y * k]);
  const r = RADIUS * k;
  const buf = Buffer.alloc(big * big * 4);
  for (let y = 0; y < big; y++) {
    for (let x = 0; x < big; x++) {
      const px = x + 0.5;
      const py = y + 0.5;
      let c = BG;
      if (pointInPolygon(px, py, poly)) c = FG;
      else if (!inRoundedRect(px, py, big, r)) {
        buf[(y * big + x) * 4 + 3] = 0;
        continue;
      }
      const i = (y * big + x) * 4;
      buf[i] = c[0];
      buf[i + 1] = c[1];
      buf[i + 2] = c[2];
      buf[i + 3] = 255;
    }
  }
  // Box downsample.
  const out = Buffer.alloc(size * size * 4);
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      let rS = 0,
        gS = 0,
        bS = 0,
        aS = 0;
      for (let dy = 0; dy < SS; dy++) {
        for (let dx = 0; dx < SS; dx++) {
          const i = ((y * SS + dy) * big + (x * SS + dx)) * 4;
          rS += buf[i];
          gS += buf[i + 1];
          bS += buf[i + 2];
          aS += buf[i + 3];
        }
      }
      const n = SS * SS;
      const i = (y * size + x) * 4;
      out[i] = Math.round(rS / n);
      out[i + 1] = Math.round(gS / n);
      out[i + 2] = Math.round(bS / n);
      out[i + 3] = Math.round(aS / n);
    }
  }
  return encodePng(out, size, size);
}

for (const size of [16, 32, 48, 128]) {
  const file = join(dir, `icon${size}.png`);
  writeFileSync(file, renderIcon(size));
  console.log(`wrote ${file}`);
}
