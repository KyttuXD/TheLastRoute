// Writes hunters/mimic_atlas.png: a dependency-free RGBA PNG (zlib from node).
// 64x128 because the placeholder body (a Dovelle copy) maps its UVs onto a 64x128 atlas.
import { deflateSync } from "node:zlib";
import { writeFileSync, mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const W = 64, H = 128;
const px = Buffer.alloc(W * H * 4);
for (let y = 0; y < H; y++) {
  for (let x = 0; x < W; x++) {
    const checker = ((x >> 2) + (y >> 2)) & 1;
    const stripe = (x + y) % 16 < 3;
    const c = stripe ? [255, 200, 40] : checker ? [120, 40, 160] : [70, 20, 100];
    px.set([...c, 255], (y * W + x) * 4);
  }
}

const crcTable = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
const crc32 = (buf) => {
  let c = 0xffffffff;
  for (const b of buf) c = crcTable[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
};
const chunk = (type, data) => {
  const t = Buffer.from(type, "ascii");
  const len = Buffer.alloc(4); len.writeUInt32BE(data.length);
  const crc = Buffer.alloc(4); crc.writeUInt32BE(crc32(Buffer.concat([t, data])));
  return Buffer.concat([len, t, data, crc]);
};
const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(W, 0); ihdr.writeUInt32BE(H, 4);
ihdr[8] = 8; ihdr[9] = 6; // 8-bit RGBA
const raw = Buffer.alloc((W * 4 + 1) * H);
for (let y = 0; y < H; y++) px.copy(raw, y * (W * 4 + 1) + 1, y * W * 4, (y + 1) * W * 4);
const png = Buffer.concat([
  Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
  chunk("IHDR", ihdr),
  chunk("IDAT", deflateSync(raw)),
  chunk("IEND", Buffer.alloc(0)),
]);
const out = resolve(dirname(fileURLToPath(import.meta.url)), "../hunters/mimic_atlas.png");
mkdirSync(dirname(out), { recursive: true });
writeFileSync(out, png);
console.log("wrote", out, png.length, "bytes");
