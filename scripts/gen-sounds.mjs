// 生成壳内置的柔和完成提示音（16-bit PCM 单声道 wav）到 src-tauri/resources/sounds/。
// 程序化合成（正弦 + 指数衰减包络），无版权问题；音色不满意时调参数重跑：
//   node scripts/gen-sounds.mjs
import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const SR = 22050 // 采样率
const OUT = join(dirname(fileURLToPath(import.meta.url)), '..', 'src-tauri', 'resources', 'sounds')

// 线性 attack 防爆音 + 指数衰减；尾部 60ms 淡出到 0
function envelope(t, dur, attack = 0.008, decay = 4.5) {
  const a = Math.min(1, t / attack)
  const d = Math.exp((-decay * t) / dur)
  const tail = Math.min(1, (dur - t) / 0.06)
  return a * d * Math.max(0, tail)
}

function render(dur, fn) {
  const n = Math.round(SR * dur)
  const samples = new Int16Array(n)
  for (let i = 0; i < n; i++) samples[i] = Math.max(-1, Math.min(1, fn(i / SR))) * 32767
  return samples
}

function wav(samples) {
  const header = Buffer.alloc(44)
  header.write('RIFF', 0)
  header.writeUInt32LE(36 + samples.length * 2, 4)
  header.write('WAVEfmt ', 8)
  header.writeUInt32LE(16, 16) // PCM chunk
  header.writeUInt16LE(1, 20) // PCM
  header.writeUInt16LE(1, 22) // mono
  header.writeUInt32LE(SR, 24)
  header.writeUInt32LE(SR * 2, 28)
  header.writeUInt16LE(2, 32)
  header.writeUInt16LE(16, 34)
  header.write('data', 36)
  header.writeUInt32LE(samples.length * 2, 40)
  return Buffer.concat([header, Buffer.from(samples.buffer)])
}

const TAU = Math.PI * 2

// 轻铃：E6→A6 两个音先后响起，柔和衰减
const chime = render(0.95, (t) => {
  const n1 = Math.sin(TAU * 1318.5 * t) * envelope(t, 0.95) * 0.32
  const t2 = t - 0.28
  const n2 = t2 > 0 ? Math.sin(TAU * 1760 * t2) * envelope(t2, 0.67) * 0.26 : 0
  return n1 + n2
})

// 水滴：900→380Hz 下滑音，短促轻柔
const drop = render(0.45, (t) => {
  const f = 900 - 520 * (t / 0.45)
  return Math.sin(TAU * f * t) * envelope(t, 0.45, 0.006, 6) * 0.3
})

// 柔和和弦：C5+E5+G5 叠加，缓慢衰减
const mellow = render(1.3, (t) => {
  const s =
    (Math.sin(TAU * 523.25 * t) + Math.sin(TAU * 659.25 * t) + Math.sin(TAU * 783.99 * t)) / 3
  return s * envelope(t, 1.3, 0.015, 3.5) * 0.34
})

mkdirSync(OUT, { recursive: true })
for (const [name, samples] of [
  ['chime.wav', chime],
  ['drop.wav', drop],
  ['mellow.wav', mellow],
]) {
  writeFileSync(join(OUT, name), wav(samples))
  console.log(`written ${name} (${samples.length} samples, ${(samples.length / SR).toFixed(2)}s)`)
}
