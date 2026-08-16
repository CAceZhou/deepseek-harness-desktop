<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'
  import { onMount } from 'svelte'

  type Shortcut = { ctrl: boolean; shift: boolean; alt: boolean; code: string; key: string }
  type ShellSettings = {
    zoom_step: number
    zoom_in: Shortcut
    zoom_out: Shortcut
    close_behavior: 'background' | 'quit'
  }

  // 与 Rust 端 ShellSettings::default 保持一致
  const DEFAULTS: ShellSettings = {
    zoom_step: 0.02,
    zoom_in: { ctrl: true, shift: true, alt: false, code: 'Equal', key: '+' },
    zoom_out: { ctrl: true, shift: true, alt: false, code: 'Minus', key: '_' },
    close_behavior: 'background',
  }

  let zoomIn = $state<Shortcut>({ ...DEFAULTS.zoom_in })
  let zoomOut = $state<Shortcut>({ ...DEFAULTS.zoom_out })
  let stepPct = $state(2)
  let closeBehavior = $state<'background' | 'quit'>('background')
  let recording = $state<'in' | 'out' | null>(null)
  let saving = $state(false)
  let notice = $state<{ kind: 'ok' | 'err'; text: string } | null>(null)

  onMount(async () => {
    const s = await invoke<ShellSettings>('get_shell_settings')
    applyState(s)
  })

  function applyState(s: ShellSettings) {
    zoomIn = { ...s.zoom_in }
    zoomOut = { ...s.zoom_out }
    stepPct = Math.round(s.zoom_step * 100)
    closeBehavior = s.close_behavior
  }

  const CODE_LABELS: Record<string, string> = {
    Equal: '=', Minus: '-', Comma: ',', Period: '.', Slash: '/', Backslash: '\\',
    Semicolon: ';', Quote: "'", BracketLeft: '[', BracketRight: ']', Backquote: '`',
    Space: 'Space', ArrowUp: '↑', ArrowDown: '↓', ArrowLeft: '←', ArrowRight: '→',
  }

  function shortcutLabel(sc: Shortcut): string {
    const base =
      CODE_LABELS[sc.code] ??
      (sc.code.startsWith('Key')
        ? sc.code.slice(3)
        : sc.code.startsWith('Digit')
          ? sc.code.slice(5)
          : sc.code || sc.key)
    return [sc.ctrl && 'Ctrl', sc.shift && 'Shift', sc.alt && 'Alt', base]
      .filter(Boolean)
      .join('+')
  }

  // 录制快捷键：捕获首个非修饰键的 keydown；Esc 取消。监听器自摘除。
  function record(which: 'in' | 'out') {
    if (recording) return
    recording = which
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault()
      e.stopPropagation()
      if (['Control', 'Shift', 'Alt', 'Meta'].includes(e.key)) return
      window.removeEventListener('keydown', onKey, true)
      recording = null
      if (e.key === 'Escape') return
      const sc: Shortcut = { ctrl: e.ctrlKey, shift: e.shiftKey, alt: e.altKey, code: e.code, key: e.key }
      if (which === 'in') zoomIn = sc
      else zoomOut = sc
    }
    window.addEventListener('keydown', onKey, true)
  }

  function sameShortcut(a: Shortcut, b: Shortcut): boolean {
    return (
      a.ctrl === b.ctrl && a.shift === b.shift && a.alt === b.alt &&
      a.code === b.code && a.key === b.key
    )
  }

  function clientValidate(): string | null {
    for (const [name, sc] of [['放大', zoomIn], ['缩小', zoomOut]] as const) {
      if (!(sc.ctrl || sc.shift || sc.alt)) return `${name}快捷键必须包含 Ctrl/Shift/Alt 中至少一个修饰键`
    }
    if (sameShortcut(zoomIn, zoomOut)) return '放大与缩小快捷键不能相同'
    return null
  }

  async function save() {
    notice = null
    const err = clientValidate()
    if (err) {
      notice = { kind: 'err', text: err }
      return
    }
    saving = true
    try {
      const next: ShellSettings = {
        zoom_step: Math.min(Math.max(stepPct, 1), 25) / 100,
        zoom_in: zoomIn,
        zoom_out: zoomOut,
        close_behavior: closeBehavior,
      }
      await invoke('set_shell_settings', { next })
      notice = { kind: 'ok', text: '已保存，立即生效' }
    } catch (e) {
      notice = { kind: 'err', text: String(e) }
    } finally {
      saving = false
    }
  }

  function resetDefaults() {
    applyState(structuredClone(DEFAULTS))
    notice = { kind: 'ok', text: '已恢复默认值，点击保存生效' }
  }
</script>

<main>
  <header>
    <h1>其它设置</h1>
  </header>

  <section class="card">
    <h2>界面缩放</h2>
    <div class="row">
      <span>每次放大/缩小比例</span>
      <span class="step-input">
        <input type="number" min="1" max="25" bind:value={stepPct} /> %
      </span>
    </div>
    <div class="row">
      <span>放大快捷键</span>
      <button class="recorder" class:recording={recording === 'in'} onclick={() => record('in')}>
        {recording === 'in' ? '按下快捷键…（Esc 取消）' : shortcutLabel(zoomIn)}
      </button>
    </div>
    <div class="row">
      <span>缩小快捷键</span>
      <button class="recorder" class:recording={recording === 'out'} onclick={() => record('out')}>
        {recording === 'out' ? '按下快捷键…（Esc 取消）' : shortcutLabel(zoomOut)}
      </button>
    </div>
  </section>

  <section class="card">
    <h2>关闭主窗口时</h2>
    <label class="radio">
      <input type="radio" bind:group={closeBehavior} value="background" />
      保持后台运行（最小化到托盘）
    </label>
    <label class="radio">
      <input type="radio" bind:group={closeBehavior} value="quit" />
      退出程序
    </label>
  </section>

  {#if notice}
    <p class="notice" class:err={notice.kind === 'err'}>{notice.text}</p>
  {/if}

  <section class="actions">
    <button class="primary" onclick={save} disabled={saving}>{saving ? '保存中…' : '保存'}</button>
    <button class="ghost" onclick={resetDefaults}>恢复默认</button>
  </section>
</main>

<style>
  main {
    padding: 20px 24px;
    height: 100%;
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    gap: 14px;
    overflow-y: auto;
  }
  header {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  h1 {
    font-size: 18px;
    margin: 0;
  }
  h2 {
    font-size: 13px;
    color: #9aa3b2;
    margin: 0;
    font-weight: 600;
  }
  .card {
    background: #171b26;
    border: 1px solid #232838;
    border-radius: 10px;
    padding: 12px 16px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 13px;
    color: #9aa3b2;
  }
  .step-input {
    display: flex;
    align-items: center;
    gap: 6px;
    color: #e6e8ee;
  }
  input[type='number'] {
    width: 64px;
    background: #0a0c10;
    border: 1px solid #232838;
    border-radius: 6px;
    color: #e6e8ee;
    padding: 6px 8px;
    font-size: 13px;
  }
  .recorder {
    min-width: 170px;
    text-align: center;
    background: #0a0c10;
    border: 1px solid #232838;
    border-radius: 6px;
    color: #e6e8ee;
    padding: 6px 12px;
    font-size: 13px;
    cursor: pointer;
  }
  .recorder.recording {
    border-color: #1565c0;
    color: #64a3e8;
  }
  .radio {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    color: #9aa3b2;
    cursor: pointer;
  }
  .notice {
    margin: 0;
    font-size: 12px;
    color: #4ac26b;
  }
  .notice.err {
    color: #ef5350;
  }
  .actions {
    display: flex;
    gap: 12px;
  }
  .actions button {
    border: none;
    border-radius: 8px;
    padding: 8px 18px;
    font-size: 13px;
    cursor: pointer;
  }
  .actions button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .primary {
    background: #1565c0;
    color: #fff;
  }
  .ghost {
    background: transparent;
    border: 1px solid #232838 !important;
    color: #9aa3b2;
  }
</style>
