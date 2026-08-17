<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'
  import { onMount } from 'svelte'
  import { t } from '../i18n'

  type Shortcut = { ctrl: boolean; shift: boolean; alt: boolean; code: string; key: string }
  type CompletionSound = 'silent' | 'default' | 'im' | 'mail' | 'reminder' | 'sms' | 'chime' | 'drop' | 'mellow'
  type NotifyTiming = 'background' | 'always'
  type NotifyRule = { enabled: boolean; timing: NotifyTiming }
  // 与 Rust 端 NotifySettings 对应：approval=任务确认 question=选项选择 turn_done=任务完成
  type NotifySettings = { approval: NotifyRule; question: NotifyRule; turn_done: NotifyRule }
  type ShellSettings = {
    zoom_step: number
    zoom_in: Shortcut
    zoom_out: Shortcut
    close_behavior: 'background' | 'quit'
    notify: NotifySettings
    completion_sound: CompletionSound
  }

  // 与 Rust 端 ShellSettings::default 保持一致（三类通知默认均开、仅后台时提醒）
  const DEFAULT_RULE: NotifyRule = { enabled: true, timing: 'background' }
  const DEFAULTS: ShellSettings = {
    zoom_step: 0.02,
    zoom_in: { ctrl: true, shift: true, alt: false, code: 'Equal', key: '+' },
    zoom_out: { ctrl: true, shift: true, alt: false, code: 'Minus', key: '_' },
    close_behavior: 'background',
    notify: { approval: { ...DEFAULT_RULE }, question: { ...DEFAULT_RULE }, turn_done: { ...DEFAULT_RULE } },
    completion_sound: 'default',
  }

  // label 存中文原文，模板里经 t() 渲染——locale 切换时选项文字同步更新
  const SOUND_OPTIONS: { value: CompletionSound; label: string }[] = [
    { value: 'silent', label: '无提示音' },
    { value: 'default', label: '系统默认' },
    { value: 'im', label: '消息' },
    { value: 'mail', label: '邮件' },
    { value: 'reminder', label: '提醒' },
    { value: 'sms', label: '短信' },
    { value: 'chime', label: '轻铃' },
    { value: 'drop', label: '水滴' },
    { value: 'mellow', label: '和弦' },
  ]

  // label 存中文原文，模板里经 t() 渲染——locale 切换时选项文字同步更新
  const NOTIFY_ROWS: { key: keyof NotifySettings; label: string }[] = [
    { key: 'approval', label: '任务确认' },
    { key: 'question', label: '选项选择' },
    { key: 'turn_done', label: '任务完成' },
  ]

  let zoomIn = $state<Shortcut>({ ...DEFAULTS.zoom_in })
  let zoomOut = $state<Shortcut>({ ...DEFAULTS.zoom_out })
  let stepPct = $state(2)
  let closeBehavior = $state<'background' | 'quit'>('background')
  let notify = $state<NotifySettings>(structuredClone(DEFAULTS.notify))
  let completionSound = $state<CompletionSound>('default')
  let autostart = $state(false)
  let recording = $state<'in' | 'out' | null>(null)
  let saving = $state(false)
  let notice = $state<{ kind: 'ok' | 'err'; text: string } | null>(null)

  onMount(async () => {
    const [s, auto] = await Promise.all([
      invoke<ShellSettings>('get_shell_settings'),
      invoke<boolean>('get_autostart'),
    ])
    applyState(s)
    autostart = auto
  })

  function applyState(s: ShellSettings) {
    zoomIn = { ...s.zoom_in }
    zoomOut = { ...s.zoom_out }
    stepPct = Math.round(s.zoom_step * 100)
    closeBehavior = s.close_behavior
    notify = structuredClone(s.notify)
    completionSound = s.completion_sound
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
    for (const [name, sc] of [[t('放大'), zoomIn], [t('缩小'), zoomOut]] as const) {
      if (!(sc.ctrl || sc.shift || sc.alt)) {
        return `${name}${t('快捷键必须包含 Ctrl/Shift/Alt 中至少一个修饰键')}`
      }
    }
    if (sameShortcut(zoomIn, zoomOut)) return t('放大与缩小快捷键不能相同')
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
        notify,
        completion_sound: completionSound,
      }
      await invoke('set_shell_settings', { next })
      await invoke('set_autostart', { enabled: autostart })
      notice = { kind: 'ok', text: t('已保存，立即生效') }
    } catch (e) {
      notice = { kind: 'err', text: String(e) }
    } finally {
      saving = false
    }
  }

  function resetDefaults() {
    applyState(structuredClone(DEFAULTS))
    autostart = false
    notice = { kind: 'ok', text: t('已恢复默认值，点击保存生效') }
  }

  // 试听：toast 的音效是它的属性，只能连同通知一起听
  async function previewSound() {
    try {
      await invoke('preview_completion_sound', { sound: completionSound })
    } catch (e) {
      notice = { kind: 'err', text: String(e) }
    }
  }
</script>

<main>
  <header>
    <h1>{t('其它设置')}</h1>
  </header>

  <section class="card">
    <h2>{t('通用')}</h2>
    <label class="check">
      <input type="checkbox" bind:checked={autostart} />
      {t('开机时自动启动')}
    </label>
    <div class="divider"></div>
    <span class="group-label">{t('关闭主窗口时')}</span>
    <label class="check">
      <input type="radio" bind:group={closeBehavior} value="background" />
      {t('最小化到托盘')}
    </label>
    <label class="check">
      <input type="radio" bind:group={closeBehavior} value="quit" />
      {t('退出程序')}
    </label>
  </section>

  <section class="card">
    <h2>{t('通知提醒')}</h2>
    {#each NOTIFY_ROWS as row (row.key)}
      {@const rule = notify[row.key]}
      <div class="row">
        <label class="check">
          <input type="checkbox" bind:checked={rule.enabled} />
          {t(row.label)}
        </label>
        <span class="control">
          <select bind:value={rule.timing} disabled={!rule.enabled}>
            <option value="background">{t('仅后台时提醒')}</option>
            <option value="always">{t('总是提醒')}</option>
          </select>
        </span>
      </div>
    {/each}
    <p class="hint">{t('后台 = 本应用窗口均未聚焦；正在前台操作时不会弹通知打扰')}</p>
    <div class="divider"></div>
    <div class="row">
      <span>{t('完成提示音')}</span>
      <span class="control">
        <select bind:value={completionSound} disabled={!notify.turn_done.enabled}>
          {#each SOUND_OPTIONS as opt (opt.value)}
            <option value={opt.value}>{t(opt.label)}</option>
          {/each}
        </select>
        <button
          class="ghost small"
          onclick={previewSound}
          disabled={!notify.turn_done.enabled || completionSound === 'silent'}
        >
          {t('试听')}
        </button>
      </span>
    </div>
  </section>

  <section class="card">
    <h2>{t('界面缩放')}</h2>
    <div class="row">
      <span>{t('每次放大/缩小比例')}</span>
      <span class="control">
        <input type="number" min="1" max="25" bind:value={stepPct} /> %
      </span>
    </div>
    <div class="row">
      <span>{t('放大快捷键')}</span>
      <button class="recorder" class:recording={recording === 'in'} onclick={() => record('in')}>
        {recording === 'in' ? t('按下快捷键…（Esc 取消）') : shortcutLabel(zoomIn)}
      </button>
    </div>
    <div class="row">
      <span>{t('缩小快捷键')}</span>
      <button class="recorder" class:recording={recording === 'out'} onclick={() => record('out')}>
        {recording === 'out' ? t('按下快捷键…（Esc 取消）') : shortcutLabel(zoomOut)}
      </button>
    </div>
  </section>

  <section class="actions">
    <button class="primary" onclick={save} disabled={saving}>{saving ? t('保存中…') : t('保存')}</button>
    <button class="ghost" onclick={resetDefaults}>{t('恢复默认')}</button>
    {#if notice}
      <p class="notice" class:err={notice.kind === 'err'}>{notice.text}</p>
    {/if}
  </section>
</main>

<style>
  main {
    padding: 20px 24px;
    height: 100%;
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    gap: 16px;
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
    font-size: 12px;
    letter-spacing: 0.08em;
    color: var(--text-2);
    margin: 0;
    font-weight: 600;
  }
  .card {
    background: var(--bg-raise);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 16px 18px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    min-height: 36px;
    font-size: 13px;
    color: var(--text-2);
  }
  .control {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text);
  }
  input[type='number'] {
    width: 64px;
    height: 32px;
    box-sizing: border-box;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--text);
    padding: 6px 8px;
    font-size: 13px;
  }
  select {
    height: 32px;
    box-sizing: border-box;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--text);
    padding: 6px 8px;
    font-size: 13px;
  }
  select:disabled {
    opacity: 0.5;
  }
  .ghost.small {
    height: 32px;
    box-sizing: border-box;
    padding: 0 12px;
    font-size: 12px;
  }
  .recorder {
    min-width: 170px;
    height: 32px;
    box-sizing: border-box;
    text-align: center;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--text);
    padding: 6px 12px;
    font-size: 13px;
    cursor: pointer;
  }
  .recorder.recording {
    border-color: var(--accent);
    color: var(--accent-soft);
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    color: var(--text-2);
    cursor: pointer;
  }
  .divider {
    height: 1px;
    background: var(--border);
  }
  .group-label {
    font-size: 13px;
    color: var(--text-2);
  }
  .hint {
    margin: 0;
    font-size: 12px;
    color: var(--text-2);
    opacity: 0.8;
  }
  .notice {
    margin: 0;
    font-size: 12px;
    color: var(--ok);
  }
  .notice.err {
    color: var(--bad);
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 12px;
    border-top: 1px solid var(--border);
    padding-top: 16px;
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
    background: var(--accent);
    color: #fff;
  }
  .ghost {
    background: transparent;
    border: 1px solid var(--border) !important;
    color: var(--text-2);
  }
</style>
