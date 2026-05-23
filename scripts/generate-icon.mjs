import { deflateSync } from "node:zlib";
import { writeFileSync } from "node:fs";

const width = 256;
const height = 256;
const rgba = Buffer.alloc((width * 4 + 1) * height);

for (let y = 0; y < height; y += 1) {
  const row = y * (width * 4 + 1);
  rgba[row] = 0;
  for (let x = 0; x < width; x += 1) {
    const idx = row + 1 + x * 4;
    const cx = x - 128;
    const cy = y - 128;
    const radius = Math.sqrt(cx * cx + cy * cy);
    const inRing = radius > 78 && radius < 106;
    const inPort = x > 72 && x < 184 && y > 104 && y < 152;
    const inPinA = x > 92 && x < 108 && y > 72 && y < 105;
    const inPinB = x > 148 && x < 164 && y > 72 && y < 105;
    const inSignal = Math.abs(cy) < 9 && x > 58 && x < 198;

    if (inRing || inPort || inPinA || inPinB || inSignal) {
      rgba[idx] = 132;
      rgba[idx + 1] = 244;
      rgba[idx + 2] = 200;
      rgba[idx + 3] = 255;
    } else {
      const shade = Math.max(8, 22 - Math.floor(radius / 18));
      rgba[idx] = shade;
      rgba[idx + 1] = shade + 6;
      rgba[idx + 2] = shade + 14;
      rgba[idx + 3] = radius < 118 ? 255 : 0;
    }
  }
}

const signature = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
const ihdr = chunk("IHDR", Buffer.concat([u32(width), u32(height), Buffer.from([8, 6, 0, 0, 0])]));
const idat = chunk("IDAT", deflateSync(rgba));
const iend = chunk("IEND", Buffer.alloc(0));

writeFileSync("src-tauri/icons/icon.png", Buffer.concat([signature, ihdr, idat, iend]));

function chunk(type, data) {
  const typeBytes = Buffer.from(type);
  return Buffer.concat([u32(data.length), typeBytes, data, u32(crc32(Buffer.concat([typeBytes, data])))]);
}

function u32(value) {
  const buffer = Buffer.alloc(4);
  buffer.writeUInt32BE(value >>> 0);
  return buffer;
}

function crc32(buffer) {
  let crc = 0xffffffff;
  for (const byte of buffer) {
    crc ^= byte;
    for (let i = 0; i < 8; i += 1) {
      crc = crc & 1 ? (crc >>> 1) ^ 0xedb88320 : crc >>> 1;
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

