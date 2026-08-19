<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'
  import { onMount } from 'svelte'
  import { t } from '../i18n'

  type PluginRow = { name: string; version: string; isBundle: boolean }
  type PluginStatus = { pnpmReady: boolean; pnpmVersion: string | null; profileReady: boolean }
  type SearchResult = { name: string; version: string; description: string; installed: boolean }
  type OpResult = { exitCode: number; output: string }

  let rows = $state<PluginRow[]>([])
  let status = $state<PluginStatus | null>(null)
  let notice = $state<{ kind: 'ok' | 'err'; text: string } | null>(null)
  let busy = $state(false) // 装/卸/更新任一操作进行中
  let restarting = $state(false)
  let lastOp = $state<OpResult | null>(null) // 最近一次操作输出（可折叠展示）

  // 搜索
  let query = $state('')
  let searching = $state(false)
  let results = $state<SearchResult[]>([])
  let timer: ReturnType<typeof setTimeout> | null = null
  function onSearchInput() {
    if (timer) clearTimeout(timer)
    timer = setTimeout(runSearch, 300) // 防抖
  }
  async function runSearch() {
    const q = query.trim()
    if (q.length < 2) {
      results = []
      return
    }
    searching = true
    try {
      results = await invoke<SearchResult[]>('search_plugins', { query: q })
    } catch (e) {
      results = []
      notice = { kind: 'err', text: String(e) }
    } finally {
      searching = false
    }
  }

  async function load() {
    try {
      status = await invoke<PluginStatus>('get_plugin_status')
      rows = await invoke<PluginRow[]>('list_plugins')
    } catch (e) {
      notice = { kind: 'err', text: String(e) }
    }
  }

  async function op(label: string, fn: () => Promise<OpResult>) {
    if (busy) return
    busy = true
    notice = null
    try {
      const r = await fn()
      lastOp = r
      if (r.exitCode === 0) {
        notice = { kind: 'ok', text: t('完成，重启 dsh 后生效', { label }) }
      } else {
        notice = { kind: 'err', text: `${label}${t('失败，退出码', { code: r.exitCode })}，${t('看下方输出')}` }
      }
      await load()
      await runSearch()
    } catch (e) {
      notice = { kind: 'err', text: String(e) }
    } finally {
      busy = false
    }
  }
  const install = (spec: string) => op(t('安装'), () => invoke<OpResult>('install_plugin', { spec }))
  const uninstall = (name: string) =>
    confirm(t('卸载确认', { name })) &&
    op(t('卸载'), () => invoke<OpResult>('uninstall_plugin', { name }))
  const updateAll = () => op(t('更新全部'), () => invoke<OpResult>('update_plugins'))
  async function restartDsh() {
    restarting = true
    try {
      await invoke('restart_dsh')
    } catch (e) {
      notice = { kind: 'err', text: String(e) }
    } finally {
      restarting = false
    }
  }

  onMount(load)
</script>

<main>
  <header>
    <h1>{t('插件管理')}</h1>
    <div class="actions">
      <button class="ghost" onclick={restartDsh} disabled={busy || restarting}>
        {restarting ? t('重启中…') : t('重启 dsh')}
      </button>
      <button class="primary" onclick={updateAll} disabled={busy}>{t('更新全部')}</button>
    </div>
  </header>

  <p class="tip">
    {status?.pnpmReady
      ? t('pnpm 就绪', { version: status.pnpmVersion ?? '?' })
      : t('pnpm 缺失')}
    · {t('插件安全提示')}
  </p>

  <section class="card search">
    <div class="search-box">
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/></svg>
      <input
        placeholder={t('搜索 npm 包…')}
        bind:value={query}
        oninput={onSearchInput}
        disabled={busy}
      />
    </div>
    {#if searching}
      <div class="results">
        <p class="empty">{t('搜索中…')}</p>
      </div>
    {:else if results.length}
      <div class="results">
        {#each results as r (r.name)}
          <div class="row">
            <span class="cube">⬡</span>
            <span class="meta">
              <b>
                {r.name}
                {#if r.version}<span class="ver">v{r.version}</span>{/if}
              </b>
              <span class="desc">{r.description || t('（无描述）')}</span>
            </span>
            <button class="primary install" disabled={busy || r.installed} onclick={() => install(r.name)}>
              {r.installed ? t('已安装') : t('安装')}
            </button>
          </div>
        {/each}
      </div>
    {/if}
  </section>

  <section class="card list">
    <h2>{t('已安装')}</h2>
    {#if rows.length === 0}
      <p class="empty">{t('尚无插件，搜索并安装一个。')}</p>
    {:else}
      {#each rows as r (r.name)}
        <div class="row">
          <span class="badge">{r.isBundle ? t('插件') : t('依赖')}</span>
          <span class="meta">
            <b>{r.name}</b>
            <span class="desc">v{r.version}</span>
          </span>
          <button class="trash" title={t('卸载')} disabled={busy} onclick={() => uninstall(r.name)}>
            <svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/><path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
          </button>
        </div>
      {/each}
    {/if}
  </section>

  {#if notice}
    <p class="notice" class:err={notice.kind === 'err'}>{notice.text}</p>
  {/if}

  {#if lastOp}
    <details class="oplog">
      <summary>
        {t('操作输出')}（{lastOp.exitCode === 0 ? t('成功') : t('失败')}，{t('退出码')} {lastOp.exitCode}）
      </summary>
      <pre>{lastOp.output}</pre>
    </details>
  {/if}
</main>

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
  h2 {
    margin: 0;
    padding: 12px 16px 4px;
    font-size: 12px;
    font-weight: 600;
    color: var(--text-2);
  }
  .actions {
    display: flex;
    gap: 10px;
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
  .tip {
    margin: 0;
    font-size: 12px;
    color: var(--text-3);
  }
  .notice {
    margin: 0;
    font-size: 12px;
    color: var(--ok);
  }
  .notice.err {
    color: var(--bad);
  }
  .card {
    background: var(--bg-raise);
    border: 1px solid var(--border);
    border-radius: 10px;
  }
  .search {
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px;
  }
  .search-box {
    position: relative;
    display: flex;
    align-items: center;
  }
  .search-box svg {
    position: absolute;
    left: 9px;
    color: var(--text-3);
    pointer-events: none;
  }
  .search-box input {
    width: 100%;
    height: 32px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--text);
    padding: 6px 8px 6px 30px;
    font-size: 13px;
    box-sizing: border-box;
  }
  .results {
    border: 1px solid var(--border);
    border-radius: 8px;
    max-height: 200px;
    overflow-y: auto;
  }
  .list {
    flex: 1;
    min-height: 0;
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
  .cube {
    color: var(--text-4);
    font-size: 18px;
    width: 24px;
    text-align: center;
    flex-shrink: 0;
  }
  .badge {
    flex-shrink: 0;
    width: 48px;
    text-align: center;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.5px;
    color: var(--text-2);
    background: var(--bg-track);
    border-radius: 4px;
    padding: 3px 0;
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
    display: flex;
    align-items: center;
    gap: 8px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ver {
    font-size: 10px;
    font-weight: 400;
    color: var(--text-3);
    background: var(--bg-track);
    border-radius: 4px;
    padding: 1px 6px;
    flex-shrink: 0;
  }
  .desc {
    font-size: 12px;
    color: var(--text-2);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .install {
    flex-shrink: 0;
    min-width: 64px;
    padding: 6px 14px;
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
  .oplog {
    flex-shrink: 0;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg-raise);
    max-height: 140px;
    overflow-y: auto;
  }
  .oplog summary {
    padding: 8px 12px;
    font-size: 12px;
    color: var(--text-2);
    cursor: pointer;
    user-select: none;
  }
  .oplog pre {
    margin: 0;
    padding: 0 12px 10px;
    font-size: 11px;
    line-height: 1.5;
    color: var(--text-2);
    white-space: pre-wrap;
    word-break: break-all;
  }
</style>
