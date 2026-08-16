import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

export type UiSnapshot = { theme: 'dark' | 'light'; locale: 'zh' | 'en' }

/**
 * 壳界面状态（响应式）：主题与语言都来自 dsh 的 settings.yaml，
 * 由 Rust 侧 theme 关注循环 2s 轮询后广播；本地页面全部以此为单一事实来源。
 * 默认深色+中文——首启 splash 阶段壳还没解析出设置，事件到达后自动校正。
 */
const state = $state<UiSnapshot>({ theme: 'dark', locale: 'zh' })

export function uiState() {
  return state
}

function apply(snap: UiSnapshot) {
  state.theme = snap.theme
  state.locale = snap.locale
  const root = document.documentElement
  root.dataset.theme = snap.theme
  root.style.colorScheme = snap.theme
  root.lang = snap.locale === 'zh' ? 'zh-CN' : 'en'
}

/** 页面入口调用：先取快照（壳可能还没就绪，失败则保持默认），再订阅增量更新 */
export async function initUi() {
  try {
    apply(await invoke<UiSnapshot>('get_shell_ui_state'))
  } catch {
    // 壳未就绪：保持默认，事件到达后校正
  }
  await listen<UiSnapshot>('shell-ui-state', (e) => apply(e.payload))
}
