// Mock ssh（SSH 反向隧道测试用）：工作目录下存在 fake-ssh.fail 时把其内容打到
// stderr 并退出 1（模拟 Permission denied / remote port forwarding failed 等
// 鉴权/转发错误）；否则保持存活（模拟隧道建立成功，同真实 ssh -N）。
const fs = require('node:fs')
const path = require('node:path')

const marker = path.join(process.cwd(), 'fake-ssh.fail')
if (fs.existsSync(marker)) {
  process.stderr.write(fs.readFileSync(marker, 'utf8'))
  process.exit(1)
}

// 保持进程存活（真实 ssh -N 行为）
setInterval(() => {}, 10000)
