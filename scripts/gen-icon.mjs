// 生成应用图标：纯 Node 实现 PNG 编码 + ICO 封装，无第三方依赖。
// 用法：node scripts/gen-icon.mjs
import { deflateSync } from 'node:zlib'
import { mkdirSync, writeFileSync } from 'node:fs'
import { join, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

const ICONS_DIR = join(dirname(fileURLToPath(import.meta.url)), '..', 'src-tauri', 'icons')

// ---- 极简 PNG 编码 ----
const CRC_TABLE = Array.from({ length: 256 }, (_, n) => {
  let c = n
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1
  return c >>> 0
})
function crc32(buf) {
  let c = 0xffffffff
  for (const b of buf) c = CRC_TABLE[(c ^ b) & 0xff] ^ (c >>> 8)
  return (c ^ 0xffffffff) >>> 0
}
function chunk(type, data) {
  const len = Buffer.alloc(4)
  len.writeUInt32BE(data.length)
  const body = Buffer.concat([Buffer.from(type, 'ascii'), data])
  const crc = Buffer.alloc(4)
  crc.writeUInt32BE(crc32(body))
  return Buffer.concat([len, body, crc])
}
function encodePng(size, rgba) {
  const ihdr = Buffer.alloc(13)
  ihdr.writeUInt32BE(size, 0)
  ihdr.writeUInt32BE(size, 4)
  ihdr[8] = 8 // bit depth
  ihdr[9] = 6 // color type RGBA
  const raw = Buffer.alloc(size * (size * 4 + 1))
  for (let y = 0; y < size; y++) {
    raw[y * (size * 4 + 1)] = 0 // filter none
    rgba.copy(raw, y * (size * 4 + 1) + 1, y * size * 4, (y + 1) * size * 4)
  }
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', ihdr),
    chunk('IDAT', deflateSync(raw, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ])
}

// ---- 绘制：深蓝圆角方块 + 三条白色横杠（harness 意象） ----
function roundedRect(rgba, size, x, y, w, h, r, color) {
  for (let py = y; py < y + h; py++) {
    for (let px = x; px < x + w; px++) {
      const dx = Math.max(x + r - px, 0, px - (x + w - 1 - r))
      const dy = Math.max(y + r - py, 0, py - (y + h - 1 - r))
      if (dx * dx + dy * dy <= r * r) {
        const i = (py * size + px) * 4
        rgba[i] = color[0]; rgba[i + 1] = color[1]; rgba[i + 2] = color[2]; rgba[i + 3] = color[3]
      }
    }
  }
}
function drawIcon(size) {
  const s = size / 256
  const rgba = Buffer.alloc(size * size * 4)
  const bg = [21, 101, 192, 255] // #1565C0
  const fg = [255, 255, 255, 255]
  roundedRect(rgba, size, Math.round(16 * s), Math.round(16 * s), Math.round(224 * s), Math.round(224 * s), Math.round(48 * s), bg)
  roundedRect(rgba, size, Math.round(64 * s), Math.round(72 * s), Math.round(128 * s), Math.round(24 * s), Math.round(12 * s), fg)
  roundedRect(rgba, size, Math.round(64 * s), Math.round(116 * s), Math.round(128 * s), Math.round(24 * s), Math.round(12 * s), fg)
  roundedRect(rgba, size, Math.round(64 * s), Math.round(160 * s), Math.round(88 * s), Math.round(24 * s), Math.round(12 * s), fg)
  return rgba
}

function encodeIco(rgba256) {
  // rc.exe 不接受 PNG 压缩的 ICO（RC2176），因此 ICO 内嵌真实 DIB：
  // BITMAPINFOHEADER(biHeight=2*h) + 自下而上 BGRA 像素 + 1bpp AND 掩码（全 0 = 不透明）
  const size = 256
  const header = Buffer.alloc(6)
  header.writeUInt16LE(1, 2) // type: icon
  header.writeUInt16LE(1, 4) // count: 1

  const xorSize = size * size * 4
  const andRow = size / 8
  const andSize = andRow * size
  const dib = Buffer.alloc(40 + xorSize + andSize)
  dib.writeUInt32LE(40, 0) // biSize
  dib.writeInt32LE(size, 4) // biWidth
  dib.writeInt32LE(size * 2, 8) // biHeight = XOR + AND
  dib.writeUInt16LE(1, 12) // biPlanes
  dib.writeUInt16LE(32, 14) // biBitCount
  dib.writeUInt32LE(0, 16) // biCompression = BI_RGB
  dib.writeUInt32LE(xorSize + andSize, 20) // biSizeImage
  // 自下而上逐行写入，RGB(A) -> BGR(A)
  for (let y = 0; y < size; y++) {
    const srcRow = (size - 1 - y) * size * 4
    const dstRow = 40 + y * size * 4
    for (let x = 0; x < size; x++) {
      const s = srcRow + x * 4
      const d = dstRow + x * 4
      dib[d] = rgba256[s + 2]
      dib[d + 1] = rgba256[s + 1]
      dib[d + 2] = rgba256[s]
      dib[d + 3] = rgba256[s + 3]
    }
  }
  // AND 掩码保持全 0

  const entry = Buffer.alloc(16)
  // width/height 字节 0 表示 256
  entry.writeUInt16LE(1, 4) // planes
  entry.writeUInt16LE(32, 6) // bit count
  entry.writeUInt32LE(dib.length, 8)
  entry.writeUInt16LE(22, 12) // offset = 6 + 16
  return Buffer.concat([header, entry, dib])
}

mkdirSync(ICONS_DIR, { recursive: true })
writeFileSync(join(ICONS_DIR, '32x32.png'), encodePng(32, drawIcon(32)))
writeFileSync(join(ICONS_DIR, '128x128.png'), encodePng(128, drawIcon(128)))
const png256 = encodePng(256, drawIcon(256))
writeFileSync(join(ICONS_DIR, '128x128@2x.png'), png256)
writeFileSync(join(ICONS_DIR, 'icon.ico'), encodeIco(drawIcon(256)))
console.log('icons written to', ICONS_DIR)
