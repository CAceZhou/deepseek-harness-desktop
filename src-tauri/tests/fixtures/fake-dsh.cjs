// Mock dsh：供 process/notify 的集成测试使用。
// 解析 --port，起 HTTP 服务；GET / 返回 200；/api/events.mux 返回 SSE 心跳。
// 工作目录下存在 fake-dsh.exit-after 文件时，按其毫秒数延迟自杀（模拟崩溃）。
const http = require('node:http')
const fs = require('node:fs')
const path = require('node:path')
const crypto = require('node:crypto')

const portIdx = process.argv.indexOf('--port')
const port = Number(process.argv[portIdx + 1] || 3080)

const server = http.createServer((req, res) => {
  if (req.url === '/api/events.mux' || req.url === '/api/events.host') {
    res.writeHead(200, { 'content-type': 'text/event-stream', 'cache-control': 'no-cache' })
    res.write(': connected\n\n')
    const timer = setInterval(() => {
      res.write('data: {"type":"heartbeat"}\n\n')
    }, 2000)
    // 每 10s 发一个可通知事件，供通知桥接手动验收
    const notifier = setInterval(() => {
      res.write('data: {"type":"task.complete","text":"fake done"}\n\n')
    }, 10000)
    req.on('close', () => {
      clearInterval(timer)
      clearInterval(notifier)
    })
    return
  }
  res.writeHead(200, { 'content-type': 'text/plain' })
  res.end('ok')
})

server.listen(port, '127.0.0.1', () => {
  console.log(`listening http://127.0.0.1:${port}`)
})

// WebSocket 下行流：与真实 dsh 一致，/api/events.mux 通过 WS 推送
// {"type":"server-request","method":<事件类型>,"payload":{...}} 帧。
const WS_GUID = '258EAFA5-E914-47DA-95CA-C5AB0DC85B11'
server.on('upgrade', (req, socket) => {
  if (req.url !== '/api/events.mux' && req.url !== '/api/events.host') {
    socket.destroy()
    return
  }
  const accept = crypto
    .createHash('sha1')
    .update(req.headers['sec-websocket-key'] + WS_GUID)
    .digest('base64')
  socket.write(
    'HTTP/1.1 101 Switching Protocols\r\n' +
      'Upgrade: websocket\r\n' +
      'Connection: Upgrade\r\n' +
      `Sec-WebSocket-Accept: ${accept}\r\n\r\n`,
  )
  const send = (method) => {
    const payload = Buffer.from(
      JSON.stringify({ type: 'server-request', rpcId: 'x', method, payload: {} }),
    )
    const len = payload.length
    const header = len < 126
      ? Buffer.from([0x81, len])
      : Buffer.from([0x81, 126, (len >> 8) & 0xff, len & 0xff])
    socket.write(Buffer.concat([header, payload]))
  }
  const heartbeat = setInterval(() => send('heartbeat'), 2000)
  // 每 3s 发一个待批准事件，供通知桥接验收
  const approval = setInterval(() => send('approval/requested'), 3000)
  socket.on('close', () => {
    clearInterval(heartbeat)
    clearInterval(approval)
  })
  socket.on('error', () => {})
})

const marker = path.join(process.cwd(), 'fake-dsh.exit-after')
if (fs.existsSync(marker)) {
  const ms = Number(fs.readFileSync(marker, 'utf8').trim())
  if (ms > 0) setTimeout(() => process.exit(1), ms)
}
