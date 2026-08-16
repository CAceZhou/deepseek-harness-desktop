<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'
  import { onMount } from 'svelte'
  import { SvelteSet } from 'svelte/reactivity'
  import { t } from '../i18n'

  type McpServerConfig = {
    serverName: string
    transport: string
    command: string | null
    args: string[]
    env: Record<string, string>
    cwd: string | null
    url: string | null
    headers: Record<string, string>
  }
  type McpServerRow = {
    serverName: string
    transport: string
    summary: string
    enabled: boolean
    config: McpServerConfig
  }
  type McpSourceServer = {
    name: string
    transport: string
    summary: string
    supported: boolean
    reason: string | null
    conflict: boolean
  }
  type McpSourceInfo = { id: string; label: string; exists: boolean; servers: McpSourceServer[] }
  type McpImportResult = { name: string; status: 'imported' | 'skipped' | 'error'; error: string | null }

  type EditForm = {
    serverName: string
    transport: 'stdio' | 'streamable-http'
    command: string
    argsText: string
    envText: string
    cwd: string
    url: string
    headersText: string
  }

  let rows = $state<McpServerRow[]>([])
  let loading = $state(true)
  let readOnly = $state<string | null>(null) // patch 文件不可解析时的只读提示
  let notice = $state<{ kind: 'ok' | 'err'; text: string } | null>(null)

  // 编辑/新增弹窗状态
  let showEditor = $state(false)
  let editing = $state<McpServerRow | null>(null) // null = 新增
  let form = $state<EditForm>(emptyForm())
  let saving = $state(false)

  // 导入弹窗状态
  let showImport = $state(false)
  let sources = $state<McpSourceInfo[]>([])
  let selectedSourceId = $state('')
  let checked = new SvelteSet<string>()
  let overwrite = new SvelteSet<string>()
  let importing = $state(false)

  let selectedSource = $derived(sources.find((s) => s.id === selectedSourceId))
  let importableCount = $derived(
    [...checked].filter((n) => selectedSource?.servers.find((s) => s.name === n)?.supported).length,
  )

  function emptyForm(): EditForm {
    return {
      serverName: '',
      transport: 'stdio',
      command: '',
      argsText: '',
      envText: '',
      cwd: '',
      url: '',
      headersText: '',
    }
  }

  function kvText(map: Record<string, string>): string {
    return Object.entries(map)
      .map(([k, v]) => `${k}=${v}`)
      .join('\n')
  }

  function parseKv(text: string): Record<string, string> {
    const out: Record<string, string> = {}
    for (const line of text.split('\n')) {
      const i = line.indexOf('=')
      if (i > 0) out[line.slice(0, i).trim()] = line.slice(i + 1).trim()
    }
    return out
  }

  onMount(load)

  async function load() {
    loading = true
    try {
      rows = await invoke<McpServerRow[]>('list_mcp_servers')
      readOnly = null
    } catch (e) {
      rows = []
      readOnly = String(e)
    } finally {
      loading = false
    }
  }

  async function toggle(row: McpServerRow) {
    const next = !row.enabled
    row.enabled = next // 乐观更新，失败回滚
    try {
      await invoke('set_mcp_enabled', { serverName: row.serverName, enabled: next })
    } catch (e) {
      row.enabled = !next
      notice = { kind: 'err', text: String(e) }
    }
  }

  async function remove(row: McpServerRow) {
    if (!confirm(t('删除 MCP 确认', { name: row.serverName }))) return
    try {
      await invoke('delete_mcp_server', { serverName: row.serverName })
      notice = { kind: 'ok', text: t('已删除', { name: row.serverName }) }
      await load()
    } catch (e) {
      notice = { kind: 'err', text: String(e) }
    }
  }

  function openEditor(row: McpServerRow | null) {
    notice = null
    editing = row
    form = row
      ? {
          serverName: row.config.serverName,
          transport: (row.config.transport === 'streamable-http' ? 'streamable-http' : 'stdio') as EditForm['transport'],
          command: row.config.command ?? '',
          argsText: row.config.args.join('\n'),
          envText: kvText(row.config.env),
          cwd: row.config.cwd ?? '',
          url: row.config.url ?? '',
          headersText: kvText(row.config.headers),
        }
      : emptyForm()
    showEditor = true
  }

  async function saveEditor() {
    saving = true
    try {
      const stdio = form.transport === 'stdio'
      const config: McpServerConfig = {
        serverName: form.serverName.trim(),
        transport: form.transport,
        command: stdio ? form.command.trim() || null : null,
        args: stdio ? form.argsText.split('\n').map((s) => s.trim()).filter(Boolean) : [],
        env: stdio ? parseKv(form.envText) : {},
        cwd: stdio ? form.cwd.trim() || null : null,
        url: stdio ? null : form.url.trim() || null,
        headers: stdio ? {} : parseKv(form.headersText),
      }
      await invoke('upsert_mcp_server', {
        originalName: editing?.serverName ?? null,
        config,
      })
      notice = {
        kind: 'ok',
        text: editing ? t('已保存', { name: config.serverName }) : t('已添加', { name: config.serverName }),
      }
      showEditor = false
      await load()
    } catch (e) {
      notice = { kind: 'err', text: String(e) }
    } finally {
      saving = false
    }
  }

  async function openImport() {
    notice = null
    try {
      sources = await invoke<McpSourceInfo[]>('list_mcp_import_sources')
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
      const results = await invoke<McpImportResult[]>('import_mcp_servers', {
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
    <h1>{t('MCP 管理')}</h1>
    <span class="actions">
      <button class="ghost" onclick={() => openEditor(null)} disabled={readOnly !== null}>{t('新增 Server')}</button>
      <button class="primary" onclick={openImport} disabled={readOnly !== null}>{t('从其它工具导入')}</button>
    </span>
  </header>

  <p class="tip">{t('配置写入 dsh 的 cordis.patch.yml，热重载即时生效，无需重启服务。')}</p>

  {#if readOnly}
    <p class="notice err">{readOnly}</p>
  {/if}

  <section class="card list">
    {#if loading}
      <p class="empty">{t('加载中…')}</p>
    {:else if rows.length === 0}
      <p class="empty">{t('尚无 MCP server，点击右上角「新增 Server」或「从其它工具导入」开始。')}</p>
    {:else}
      {#each rows as row (row.serverName)}
        <div class="row" class:disabled={!row.enabled}>
          <span class="badge">{row.transport === 'streamable-http' ? 'HTTP' : 'STDIO'}</span>
          <span class="meta">
            <b>{row.serverName}</b>
            <span class="desc">{row.summary || t('（无命令/地址）')}</span>
          </span>
          <span class="state">{row.enabled ? t('已启用') : t('已停用')}</span>
          <span class="switch">
            <input type="checkbox" checked={row.enabled} onchange={() => toggle(row)} />
            <span class="track"></span>
          </span>
          <button class="edit" title={t('编辑')} onclick={() => openEditor(row)}>
            <svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z"/></svg>
          </button>
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

{#if showEditor}
  <div class="overlay" onclick={() => (showEditor = false)} role="presentation">
    <div
      class="modal"
      onclick={(e) => e.stopPropagation()}
      onkeydown={(e) => e.key === 'Escape' && (showEditor = false)}
      role="dialog"
      tabindex="-1"
    >
      <h2>{editing ? t('编辑 Server', { name: editing.serverName }) : t('新增 Server')}</h2>
      <div class="field">
        <span>{t('名称')}</span>
        <input bind:value={form.serverName} placeholder={t('字母/数字/_/-，最长 32')} />
      </div>
      <div class="field">
        <span>{t('类型')}</span>
        <select bind:value={form.transport}>
          <option value="stdio">{t('stdio（本地命令）')}</option>
          <option value="streamable-http">{t('streamable-http（远程 URL）')}</option>
        </select>
      </div>
      {#if form.transport === 'stdio'}
        <div class="field">
          <span>{t('命令')}</span>
          <input bind:value={form.command} placeholder={t('如 npx')} />
        </div>
        <div class="field top">
          <span>{t('参数')}</span>
          <textarea bind:value={form.argsText} rows="3" placeholder={t('每行一个参数，如：\n-y\n@playwright/mcp')}></textarea>
        </div>
        <div class="field top">
          <span>{t('环境变量')}</span>
          <textarea bind:value={form.envText} rows="3" placeholder={t('每行一条 KEY=VALUE')}></textarea>
        </div>
        <div class="field">
          <span>{t('工作目录')}</span>
          <input bind:value={form.cwd} placeholder={t('（可空）')} />
        </div>
      {:else}
        <div class="field">
          <span>{t('URL')}</span>
          <input bind:value={form.url} placeholder="https://…/mcp" />
        </div>
        <div class="field top">
          <span>{t('请求头')}</span>
          <textarea bind:value={form.headersText} rows="3" placeholder={t('每行一条 KEY=VALUE，如：\nAuthorization=Bearer …')}></textarea>
        </div>
      {/if}
      <div class="modal-actions">
        <button class="ghost" onclick={() => (showEditor = false)}>{t('取消')}</button>
        <button class="primary" disabled={saving} onclick={saveEditor}>
          {saving ? t('保存中…') : t('保存')}
        </button>
      </div>
    </div>
  </div>
{/if}

{#if showImport}
  <div class="overlay" onclick={() => (showImport = false)} role="presentation">
    <div
      class="modal"
      onclick={(e) => e.stopPropagation()}
      onkeydown={(e) => e.key === 'Escape' && (showImport = false)}
      role="dialog"
      tabindex="-1"
    >
      <h2>{t('从其它工具导入 MCP server')}</h2>
      <div class="field">
        <span>{t('来源')}</span>
        <select bind:value={selectedSourceId}>
          {#each sources as s (s.id)}
            <option value={s.id} disabled={!s.exists}>
              {s.label}{s.exists ? `（${s.servers.length} 项）` : t('（配置不存在）')}
            </option>
          {/each}
        </select>
      </div>
      <div class="pick-list">
        {#if selectedSource && selectedSource.exists && selectedSource.servers.length > 0}
          {#each selectedSource.servers as sv (sv.name)}
            <div class="pick-row">
              <label class="check">
                <input
                  type="checkbox"
                  disabled={!sv.supported}
                  checked={checked.has(sv.name)}
                  onchange={(e) => toggleChecked(sv.name, (e.target as HTMLInputElement).checked)}
                />
                <span class="meta">
                  <b>{sv.name}</b>
                  <span class="desc">{sv.supported ? sv.summary || sv.transport : sv.reason}</span>
                </span>
              </label>
              {#if sv.conflict && checked.has(sv.name) && sv.supported}
                <select
                  class="conflict-choice"
                  value={overwrite.has(sv.name) ? 'overwrite' : 'skip'}
                  onchange={(e) => {
                    if ((e.target as HTMLSelectElement).value === 'overwrite') overwrite.add(sv.name)
                    else overwrite.delete(sv.name)
                  }}
                >
                  <option value="skip">{t('跳过')}</option>
                  <option value="overwrite">{t('覆盖')}</option>
                </select>
              {:else if sv.conflict && sv.supported}
                <span class="conflict-tag">{t('已存在')}</span>
              {/if}
            </div>
          {/each}
        {:else}
          <p class="empty">{t('该来源没有可导入的 MCP server。')}</p>
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
  .actions {
    display: flex;
    gap: 10px;
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
  .trash,
  .edit {
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
  .edit:hover {
    color: #64b5f6;
    background: rgba(100, 181, 246, 0.1);
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
    max-height: 85vh;
    overflow-y: auto;
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
  .field.top {
    align-items: flex-start;
  }
  .field > span {
    width: 64px;
    flex-shrink: 0;
  }
  select,
  input,
  textarea {
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--text);
    padding: 6px 8px;
    font-size: 13px;
    box-sizing: border-box;
  }
  select {
    height: 32px;
  }
  textarea {
    font-family: inherit;
    resize: vertical;
  }
  .field select,
  .field input,
  .field textarea {
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
