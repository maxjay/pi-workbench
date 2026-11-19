// Draws the app's icon at every size Windows asks for, straight from the 4x4 mark.
//
// Each size is drawn on whole pixels. A size made by shrinking a larger picture
// has grey edges, and the mark is only squares, so it shows.
//
//   node scripts/icons.mjs

import { writeFileSync } from 'node:fs';
import { deflateSync } from 'node:zlib';

const OUT = new URL('../app/src-tauri/icons/', import.meta.url);

// The mark's filled cells, 0 to 15, left to right, top to bottom.
const CELLS = [1, 2, 3, 4, 5, 7, 10, 14];
const PAPER = [0x0e, 0x0f, 0x0d];
const INK = [0xec, 0xeb, 0xe4];

// What Windows draws at 100%, 125%, 150%, 175%, 200% and 300%: the title bar (16 and up),
// the taskbar (24 and up), the desktop and file lists (32 and up, 48 and up), and 256.
const ICO = [16, 20, 24, 28, 30, 32, 36, 40, 42, 48, 56, 60, 64, 72, 96, 128, 256];

/** The icon as RGBA: the mark is four cells wide with about one cell around it. */
function drawn(size) {
  const cell = Math.round(size / 6);
  const edge = (size - cell * 4) / 2;
  const pixels = Buffer.alloc(size * size * 4);
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const column = Math.floor((x - edge) / cell);
      const row = Math.floor((y - edge) / cell);
      const inside = column >= 0 && column < 4 && row >= 0 && row < 4;
      const colour = inside && CELLS.includes(row * 4 + column) ? INK : PAPER;
      pixels.set([...colour, 0xff], (y * size + x) * 4);
    }
  }
  return pixels;
}

const CRC = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});

function crc(bytes) {
  let c = 0xffffffff;
  for (const byte of bytes) c = CRC[(c ^ byte) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const body = Buffer.concat([Buffer.from(type, 'latin1'), data]);
  const head = Buffer.alloc(4);
  head.writeUInt32BE(data.length);
  const tail = Buffer.alloc(4);
  tail.writeUInt32BE(crc(body));
  return Buffer.concat([head, body, tail]);
}

function png(size) {
  const pixels = drawn(size);
  const header = Buffer.alloc(13);
  header.writeUInt32BE(size, 0);
  header.writeUInt32BE(size, 4);
  header.set([8, 6, 0, 0, 0], 8); // 8 bits, RGBA
  const rows = Buffer.alloc(size * (size * 4 + 1));
  for (let y = 0; y < size; y++) {
    pixels.copy(rows, y * (size * 4 + 1) + 1, y * size * 4, (y + 1) * size * 4);
  }
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', header),
    chunk('IDAT', deflateSync(rows, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

function ico(sizes) {
  const pictures = sizes.map(png);
  const head = Buffer.alloc(6 + 16 * sizes.length);
  head.writeUInt16LE(1, 2);
  head.writeUInt16LE(sizes.length, 4);
  let at = head.length;
  sizes.forEach((size, i) => {
    const entry = 6 + 16 * i;
    head.set([size % 256, size % 256], entry); // 0 stands for 256
    head.writeUInt16LE(1, entry + 4);
    head.writeUInt16LE(32, entry + 6);
    head.writeUInt32LE(pictures[i].length, entry + 8);
    head.writeUInt32LE(at, entry + 12);
    at += pictures[i].length;
  });
  return Buffer.concat([head, ...pictures]);
}

writeFileSync(new URL('icon.ico', OUT), ico(ICO));
writeFileSync(new URL('32x32.png', OUT), png(32));
writeFileSync(new URL('128x128.png', OUT), png(128));
writeFileSync(new URL('128x128@2x.png', OUT), png(256));
writeFileSync(new URL('icon.png', OUT), png(512));
console.log(`drew ${ICO.length} sizes into icon.ico, and 4 pictures`);
