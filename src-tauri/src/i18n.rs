//! 壳界面语言：跟随 dsh 的 `locale.preference`（zh/en，缺省按系统 UI 语言）。
//! 后端文案（托盘菜单/窗口标题/进度/通知/命令错误）经 `pick` 取当前语言；
//! 当前语言由 theme 关注循环统一写入本模块的全局原子值，
//! 这样深层辅助函数（skills/mcp 校验等）无需层层透传 locale 参数。
use std::sync::atomic::{AtomicU8, Ordering};

static LOCALE: AtomicU8 = AtomicU8::new(0); // 0=zh, 1=en

pub fn set_locale(locale: &str) {
    LOCALE.store(if locale == "en" { 1 } else { 0 }, Ordering::SeqCst);
}

pub fn locale() -> &'static str {
    if LOCALE.load(Ordering::SeqCst) == 1 {
        "en"
    } else {
        "zh"
    }
}

/// 按当前语言二选一。调用点直接写双语字面量，避免维护 key 表。
pub fn pick(zh: impl Into<String>, en: impl Into<String>) -> String {
    if locale() == "en" {
        en.into()
    } else {
        zh.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_zh() {
        // 注意：不能在此 set_locale 再断言——全局原子与其它并行测试
        // （notify 等断言 zh 文案）存在竞态；切换行为由 theme 关注循环驱动
        assert_eq!(locale(), "zh");
        assert_eq!(pick("中文", "English"), "中文");
    }
}
