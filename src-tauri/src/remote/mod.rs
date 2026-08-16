//! 远程访问：Cloudflare Quick Tunnel + 壳内嵌 token 鉴权反向代理。
//! 链路与安全模型见 docs/design.zh-CN.md；概览：
//!   手机浏览器 ─HTTPS→ Cloudflare ─→ cloudflared(纯出站)
//!     → 127.0.0.1:proxy(proxy.rs，token 门岗) → 127.0.0.1:dsh(完整 Web UI)
pub mod proxy;

use rand::Rng;

/// 每次开启远程访问重新生成的会话凭据：256-bit 随机，64 字符小写 hex
pub fn generate_token() -> String {
    let mut buf = [0u8; 32];
    rand::thread_rng().fill(&mut buf);
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// 常数时间比较（等长时逐字节异或）；长度不等直接 false
pub(crate) fn token_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
