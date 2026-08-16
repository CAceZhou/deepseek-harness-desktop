use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Manager, State};

/// UI 缩放步进（加性）：Ctrl+Shift+= / Ctrl+Shift+- 每次 ±2 个百分点
pub const ZOOM_STEP: f64 = 0.02;
/// WebView2 ZoomFactor 有效区间内的安全边界
pub const ZOOM_MIN: f64 = 0.25;
pub const ZOOM_MAX: f64 = 5.0;
const FILE_NAME: &str = "ui-zoom.txt";

fn clamp_zoom(v: f64) -> f64 {
    if v.is_nan() {
        1.0
    } else {
        v.clamp(ZOOM_MIN, ZOOM_MAX)
    }
}

fn file_path(dir: &Path) -> PathBuf {
    dir.join(FILE_NAME)
}

/// 读取持久化缩放；文件缺失/损坏时回退 1.0（越界值走 clamp）
fn load(dir: &Path) -> f64 {
    std::fs::read_to_string(file_path(dir))
        .ok()
        .and_then(|s| s.trim().parse::<f64>().ok())
        .map(clamp_zoom)
        .unwrap_or(1.0)
}

/// 写盘失败不影响缩放本身（缩放是易失便利功能，下次启动回退 100% 可接受）
fn save(dir: &Path, v: f64) {
    let _ = std::fs::create_dir_all(dir).and_then(|_| std::fs::write(file_path(dir), format!("{v:.4}")));
}

pub struct ZoomState {
    dir: PathBuf,
    value: Mutex<f64>,
}

impl ZoomState {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            value: Mutex::new(load(&dir)),
            dir,
        }
    }

    pub fn get(&self) -> f64 {
        *self.value.lock().unwrap()
    }

    pub fn adjust(&self, delta: f64) -> f64 {
        let mut g = self.value.lock().unwrap();
        *g = clamp_zoom(*g + delta);
        save(&self.dir, *g);
        *g
    }

    pub fn set(&self, v: f64) -> f64 {
        let mut g = self.value.lock().unwrap();
        *g = clamp_zoom(v);
        save(&self.dir, *g);
        *g
    }
}

/// 把缩放值应用到主窗口 webview（窗口还没建好时令其静默失败，
/// 首帧 on_page_load 会统一补应用）
pub fn apply_to_main(app: &AppHandle, v: f64) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_zoom(v);
    }
}

#[tauri::command]
pub fn zoom_ui(app: AppHandle, state: State<ZoomState>, delta: f64) -> Result<(), String> {
    let v = state.adjust(delta);
    apply_to_main(&app, v);
    Ok(())
}

/// 注入每个页面（本地 splash 与远程 dsh UI 通用）的快捷键钩子。
/// 主匹配 e.code 物理键位；e.key 兜底覆盖合成按键/RDP 等 code 为空的场景
/// （Ctrl+Shift+= 在美式布局下 key 为 '+'，Ctrl+Shift+- 为 '_'）。
/// capture 阶段拦截并阻止冒泡，避免页面自身处理器重复响应。
/// 注意：脚本内步进字面量 0.02 须与 ZOOM_STEP 保持一致。
pub const HOOK_JS: &str = r#"(() => {
  if (window.__dshZoomHook) return;
  window.__dshZoomHook = true;
  window.addEventListener('keydown', (e) => {
    if (!e.ctrlKey || !e.shiftKey || e.altKey || e.metaKey) return;
    const zoomIn = e.code === 'Equal' || e.key === '+' || e.key === '=';
    const zoomOut = e.code === 'Minus' || e.key === '-' || e.key === '_';
    const delta = zoomIn ? 0.02 : zoomOut ? -0.02 : 0;
    if (!delta) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    const t = window.__TAURI__;
    const invoke = t && t.core && t.core.invoke
      ? t.core.invoke.bind(t.core)
      : window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke;
    if (invoke) invoke('zoom_ui', { delta });
  }, true);
})();
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_bounds_and_nan() {
        assert_eq!(clamp_zoom(0.01), ZOOM_MIN);
        assert_eq!(clamp_zoom(99.0), ZOOM_MAX);
        assert_eq!(clamp_zoom(f64::NAN), 1.0);
    }

    #[test]
    fn adjust_accumulates_and_clamps() {
        let dir = tempfile::tempdir().unwrap();
        let s = ZoomState::new(dir.path().to_path_buf());
        assert_eq!(s.get(), 1.0);
        let v = s.adjust(ZOOM_STEP);
        assert!((v - 1.02).abs() < 1e-9);
        assert!((s.adjust(ZOOM_STEP) - 1.04).abs() < 1e-9);
        assert_eq!(s.adjust(-10.0), ZOOM_MIN); // 大步越界被 clamp
        assert_eq!(s.get(), ZOOM_MIN);
    }

    #[test]
    fn set_persists_and_loads_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let s = ZoomState::new(dir.path().to_path_buf());
        s.set(1.04);
        let s2 = ZoomState::new(dir.path().to_path_buf());
        assert!((s2.get() - 1.04).abs() < 1e-9);
    }

    #[test]
    fn load_fallbacks() {
        let dir = tempfile::tempdir().unwrap();
        // 文件不存在 → 1.0
        assert_eq!(load(dir.path()), 1.0);
        // 垃圾内容 → 1.0
        std::fs::write(dir.path().join("ui-zoom.txt"), "abc").unwrap();
        assert_eq!(load(dir.path()), 1.0);
        // 越界值 → clamp
        std::fs::write(dir.path().join("ui-zoom.txt"), "42").unwrap();
        assert_eq!(load(dir.path()), ZOOM_MAX);
    }

    #[test]
    fn hook_js_markers() {
        assert!(HOOK_JS.contains("__dshZoomHook")); // 幂等标志
        assert!(HOOK_JS.contains("Equal") && HOOK_JS.contains("Minus"));
        assert!(HOOK_JS.contains("zoom_ui"));
        assert!(HOOK_JS.contains("ctrlKey") && HOOK_JS.contains("shiftKey"));
        // e.key 兜底：合成按键/RDP 场景 e.code 为空，Shift+= 得 '+'、Shift+- 得 '_'
        assert!(HOOK_JS.contains("e.key") && HOOK_JS.contains("'+'") && HOOK_JS.contains("'_'"));
    }
}
