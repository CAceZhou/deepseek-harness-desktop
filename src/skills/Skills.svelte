<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'
  import { onMount } from 'svelte'
  import { SvelteSet } from 'svelte/reactivity'
  import { t } from '../i18n'

  type SkillRow = { name: string; description: string; enabled: boolean }
  type SourceSkill = { name: string; description: string; conflict: boolean }
  type SourceInfo = { id: string; label: string; exists: boolean; skills: SourceSkill[] }
  type ImportResult = { name: string; status: 'imported' | 'skipped' | 'error'; error: string | null }

  let rows = $state<SkillRow[]>([])
  let loading = $state(true)
  let notice = $state<{ kind: 'ok' | 'err'; text: string } | null>(null)

  // 导入弹窗状态
  let showImport = $state(false)
  let sources = $state<SourceInfo[]>([])
  let selectedSourceId = $state('')
  let checked = new SvelteSet<string>()
  let overwrite = new SvelteSet<string>() // 冲突项里选了"覆盖"的
  let importing = $state(false)

  let selectedSource = $derived(sources.find((s) => s.id === selectedSourceId))
  let importableCount = $derived(checked.size)

  onMount(load)

  async function load() {
    loading = true
    try {
      rows = await invoke<SkillRow[]>('list_skills')
    } catch (e) {
      notice = { kind: 'err', text: String(e) }
    } finally {
      loading = false
    }
  }

  async function toggle(row: SkillRow) {
    const next = !row.enabled
    row.enabled = next // 乐观更新，失败回滚
    try {
      await invoke('set_skill_enabled', { name: row.name, enabled: next })
    } catch (e) {
      row.enabled = !next
      notice = { kind: 'err', text: String(e) }
    }
  }

  async function remove(row: SkillRow) {
    if (!confirm(t('删除技能确认', { name: row.name }))) return
    try {
      await invoke('delete_skill', { name: row.name })
      notice = { kind: 'ok', text: t('已删除', { name: row.name }) }
      await load()
    } catch (e) {
      notice = { kind: 'err', text: String(e) }
    }
  }

  async function openImport() {
    notice = null
    try {
      sources = await invoke<SourceInfo[]>('list_import_sources')
      selectedSourceId = sources.find((s) => s.exists)?.id ?? sources[0]?.id ?? ''
      checked.clear()
      overwrite.clear()
      showImport = true
    } catch (e) {
      notice = { kind: 'err', text: String(e) }
    }
  }

  function toggleChecked(name: string, on: boolean) {
    if (on) checked.add(name)
    else {
      checked.delete(name)
      overwrite.delete(name)
    }
  }

  async function confirmImport() {
    if (!selectedSource) return
    importing = true
    try {
      const items = [...checked].map((name) => ({ name, overwrite: overwrite.has(name) }))
      const results = await invoke<ImportResult[]>('import_skills', {
        sourceId: selectedSource.id,
        items,
      })
      const ok = results.filter((r) => r.status === 'imported').length
      const skipped = results.filter((r) => r.status === 'skipped').length
      const failed = results.filter((r) => r.status === 'error')
      const parts = [t('导入完成', { count: ok })]
      if (skipped) parts.push(t('跳过完成', { count: skipped }))
      if (failed.length) parts.push(t('失败完成', { count: failed.length, err: failed[0].error ?? '' }))
      notice = { kind: failed.length ? 'err' : 'ok', text: parts.join('，') }
      showImport = false
      await load()
    } catch (e) {
      notice = { kind: 'err', text: String(e) }
    } finally {
      importing = false
    }
  }
</script>

<main>
  <header>
    <h1>{t('技能管理')}</h1>
    <button class="primary" onclick={openImport}>{t('从外部 Agent 导入')}</button>
  </header>

  <p class="tip">{t('启用后可在 dsh 会话中通过技能名使用；开关即时生效，无需重启服务。')}</p>

  <section class="card list">
    {#if loading}
      <p class="empty">{t('加载中…')}</p>
    {:else if rows.length === 0}
      <p class="empty">{t('尚无技能，点击右上角「从外部 Agent 导入」开始。')}</p>
    {:else}
      {#each rows as row (row.name)}
        <div class="row" class:disabled={!row.enabled}>
          <span class="cube">⬡</span>
          <span class="meta">
            <b>{row.name}</b>
            <span class="desc">{row.description || t('（无描述）')}</span>
          </span>
          <span class="state">{row.enabled ? t('已启用') : t('已停用')}</span>
          <span class="switch">
            <input type="checkbox" checked={row.enabled} onchange={() => toggle(row)} />
            <span class="track"></span>
          </span>
          <button class="trash" title={t('删除')} onclick={() => remove(row)}>
            <svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/><path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
          </button>
        </div>
      {/each}
    {/if}
  </section>

  {#if notice}
    <p class="notice" class:err={notice.kind === 'err'}>{notice.text}</p>
  {/if}
</main>

{#if showImport}
  <div class="overlay" onclick={() => (showImport = false)} role="presentation">
    <div
      class="modal"
      onclick={(e) => e.stopPropagation()}
      onkeydown={(e) => e.key === 'Escape' && (showImport = false)}
      role="dialog"
      tabindex="-1"
    >
      <h2>{t('从外部 Agent 导入技能')}</h2>
      <div class="field">
        <span>{t('来源')}</span>
        <select bind:value={selectedSourceId}>
          {#each sources as s (s.id)}
            <option value={s.id} disabled={!s.exists}>
              {s.label}{s.exists ? `（${s.skills.length} 项）` : t('（目录不存在）')}
            </option>
          {/each}
        </select>
      </div>
      <div class="pick-list">
        {#if selectedSource && selectedSource.exists && selectedSource.skills.length > 0}
          {#each selectedSource.skills as sk (sk.name)}
            <div class="pick-row">
              <label class="check">
                <input
                  type="checkbox"
                  checked={checked.has(sk.name)}
                  onchange={(e) => toggleChecked(sk.name, (e.target as HTMLInputElement).checked)}
                />
                <span class="meta">
                  <b>{sk.name}</b>
                  <span class="desc">{sk.description || t('（无描述）')}</span>
                </span>
              </label>
              {#if sk.conflict && checked.has(sk.name)}
                <select
                  class="conflict-choice"
                  value={overwrite.has(sk.name) ? 'overwrite' : 'skip'}
                  onchange={(e) => {
                    if ((e.target as HTMLSelectElement).value === 'overwrite') overwrite.add(sk.name)
                    else overwrite.delete(sk.name)
                  }}
                >
                  <option value="skip">{t('跳过')}</option>
                  <option value="overwrite">{t('覆盖')}</option>
                </select>
              {:else if sk.conflict}
                <span class="conflict-tag">{t('已存在')}</span>
              {/if}
            </div>
          {/each}
        {:else}
          <p class="empty">{t('该来源没有可导入的技能。')}</p>
        {/if}
      </div>
      <div class="modal-actions">
        <button class="ghost" onclick={() => (showImport = false)}>{t('取消')}</button>
        <button class="primary" disabled={importing || importableCount === 0} onclick={confirmImport}>
          {importing ? t('导入中…') : t('导入完成', { count: importableCount })}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  main {
    padding: 20px 24px;
    height: 100%;
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    gap: 14px;
    overflow: hidden;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  h1 {
    font-size: 18px;
    margin: 0;
  }
  .tip {
    margin: 0;
    font-size: 12px;
    color: var(--text-3);
  }
  .card {
    background: var(--bg-raise);
    border: 1px solid var(--border);
    border-radius: 10px;
  }
  .list {
    flex: 1;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
  }
  .empty {
    margin: 0;
    padding: 24px;
    text-align: center;
    font-size: 13px;
    color: var(--text-3);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 16px;
    border-bottom: 1px solid var(--border);
  }
  .row:last-child {
    border-bottom: none;
  }
  .row:hover {
    background: var(--hover);
  }
  .row.disabled .meta {
    opacity: 0.55;
  }
  .cube {
    color: var(--text-4);
    font-size: 18px;
    width: 24px;
    text-align: center;
  }
  .meta {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .meta b {
    font-size: 13px;
    color: var(--text);
    font-weight: 600;
  }
  .desc {
    font-size: 12px;
    color: var(--text-2);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .state {
    font-size: 12px;
    color: var(--text-2);
    flex-shrink: 0;
  }
  .switch {
    position: relative;
    width: 36px;
    height: 20px;
    flex-shrink: 0;
  }
  .switch input {
    position: absolute;
    inset: 0;
    margin: 0;
    opacity: 0;
    cursor: pointer;
    z-index: 1;
  }
  .switch .track {
    position: absolute;
    inset: 0;
    background: var(--bg-track);
    border-radius: 10px;
    transition: background 0.15s;
  }
  .switch .track::after {
    content: '';
    position: absolute;
    top: 2px;
    left: 2px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: var(--text-2);
    transition:
      transform 0.15s,
      background 0.15s;
  }
  .switch input:checked + .track {
    background: var(--accent);
  }
  .switch input:checked + .track::after {
    transform: translateX(16px);
    background: #fff;
  }
  .trash {
    background: transparent;
    border: none;
    color: var(--text-3);
    cursor: pointer;
    padding: 6px;
    border-radius: 6px;
    display: flex;
  }
  .trash:hover {
    color: var(--bad);
    background: var(--bad-soft-bg);
  }
  .notice {
    margin: 0;
    font-size: 12px;
    color: var(--ok);
  }
  .notice.err {
    color: var(--bad);
  }
  .overlay {
    position: fixed;
    inset: 0;
    background: var(--overlay);
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .modal {
    width: 560px;
    max-height: 80vh;
    background: var(--bg-raise);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 18px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .modal h2 {
    margin: 0;
    font-size: 15px;
  }
  .field {
    display: flex;
    align-items: center;
    gap: 12px;
    font-size: 13px;
    color: var(--text-2);
  }
  select {
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--text);
    padding: 6px 8px;
    font-size: 13px;
    height: 32px;
    box-sizing: border-box;
  }
  .field select {
    flex: 1;
  }
  .pick-list {
    border: 1px solid var(--border);
    border-radius: 8px;
    overflow-y: auto;
    max-height: 320px;
  }
  .pick-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
  }
  .pick-row:last-child {
    border-bottom: none;
  }
  .check {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 10px;
    cursor: pointer;
  }
  .conflict-tag {
    font-size: 12px;
    color: var(--warn);
    flex-shrink: 0;
  }
  .conflict-choice {
    flex-shrink: 0;
  }
  .modal-actions {
    display: flex;
    justify-content: flex-end;
    gap: 12px;
  }
  button {
    border: none;
    border-radius: 8px;
    padding: 8px 18px;
    font-size: 13px;
    cursor: pointer;
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .primary {
    background: var(--accent);
    color: #fff;
  }
  .ghost {
    background: transparent;
    border: 1px solid var(--border);
    color: var(--text-2);
  }
</style>
