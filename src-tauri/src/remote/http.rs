//! 明文 HTTP 支持的独立开关与实现（`allow_http`，默认关闭）。
//!
//! 远程访问默认只走 HTTPS：Cloudflare 隧道天然 HTTPS（trycloudflare 域名）；
//! 自定义远程（SSH 反向隧道）默认生成的链接也是 `https://`，由服务器侧 TLS
//! 反代承载。明文 HTTP（局域网直连、或 SSH 暴露端口非 TLS 直吐）是**可选**能力：
//! 用户显式开启 `allow_http` 后才可用。
//!
//! 这里的每一块都是"HTTP 支持"的落地，全部由开关门控：
//! - `SECURE_CONTEXT_POLYFILL`：明文 HTTP 是非 secure context，dsh 前端依赖的
//!   `crypto.randomUUID()` / `navigator.clipboard` 缺失，页面会崩；仅开启时注入。
//!   关闭时 HTTPS 本就是 secure context，无需 polyfill。
//! - `cookie_attributes`：关闭时种 Secure cookie——浏览器在 http 下拒存拒发，
//!   鉴权整链直接断掉，即"默认不支持 HTTP"的落地；开启时去掉 Secure。
//! - `link_scheme`：关闭时访问链接一律 `https://`；开启时跟随服务器地址前缀。

/// secure-context polyfill：`crypto.randomUUID()` 只在 secure context（HTTPS/
/// localhost）下存在，明文 HTTP 下 undefined，一建消息/会话就抛
/// "crypto.randomUUID is not a function"，远程端整个界面崩掉（0.1.19 换掉
/// HTTPS 隧道后回归即此因）。`crypto.getRandomValues` 非 secure context 可用，
/// 用它实现 RFC 4122 v4。`navigator.clipboard` 同理补 writeText（execCommand
/// 兜底，失败仅复制不可用）。只经代理注入，桌面本机访问 127.0.0.1（本就是
/// secure context）不受影响；dsh 上游自带 polyfill 后此块失效无害。必须注入到
/// `<head>` 开头：dsh 脚本全是 module（defer，文档解析完才执行），内联同步脚本
/// 先跑即可兜住；放到 head 开头最稳（含可能的同步脚本）。
pub const SECURE_CONTEXT_POLYFILL: &str = r#"(function () {
  'use strict';
  var c = window.crypto;
  if (c && typeof c.randomUUID !== 'function' && typeof c.getRandomValues === 'function') {
    c.randomUUID = function () {
      var b = c.getRandomValues(new Uint8Array(16));
      b[6] = (b[6] & 0x0f) | 0x40;
      b[8] = (b[8] & 0x3f) | 0x80;
      var h = '';
      for (var i = 0; i < 16; i++) {
        var x = b[i].toString(16);
        if (x.length === 1) x = '0' + x;
        h += x;
      }
      return h.slice(0, 8) + '-' + h.slice(8, 12) + '-' + h.slice(12, 16) + '-' + h.slice(16, 20) + '-' + h.slice(20);
    };
  }
  if (typeof navigator !== 'undefined' && navigator.clipboard === undefined) {
    try {
      Object.defineProperty(navigator, 'clipboard', {
        configurable: true,
        value: {
          writeText: function (text) {
            var ta = document.createElement('textarea');
            ta.value = String(text);
            ta.style.position = 'fixed';
            ta.style.opacity = '0';
            document.body.appendChild(ta);
            ta.focus();
            ta.select();
            var ok = false;
            try { ok = document.execCommand('copy'); } catch (e) { ok = false; }
            document.body.removeChild(ta);
            return ok ? Promise.resolve() : Promise.reject(new Error('copy unavailable'));
          },
          readText: function () { return Promise.reject(new Error('clipboard read unavailable')); }
        }
      });
    } catch (e) {}
  }
})();"#;

/// 种 cookie 的属性串。关闭时带 Secure：明文 HTTP 下浏览器拒存/拒发该 cookie，
/// 鉴权链在 http 访问时直接断掉（403 门页）——即"默认不支持 HTTP"；开启时去掉
/// Secure，局域网直连等明文链路可用。token 本身就是链接凭据，明文网络的暴露
/// 窗口靠"局域网信任 + 每次开启轮换 + 泄露即重置吊销"兜底（见 design 文档）。
pub fn cookie_attributes(allow_http: bool) -> &'static str {
    if allow_http {
        "HttpOnly; SameSite=Lax; Path=/"
    } else {
        "HttpOnly; Secure; SameSite=Lax; Path=/"
    }
}

/// 访问链接的协议：关闭时一律 https（自定义远程默认不支持 HTTP）；开启时跟随
/// 服务器地址前缀（https:// → https，否则 http）。
pub fn link_scheme(allow_http: bool, https_prefix: bool) -> &'static str {
    if !allow_http {
        "https"
    } else if https_prefix {
        "https"
    } else {
        "http"
    }
}