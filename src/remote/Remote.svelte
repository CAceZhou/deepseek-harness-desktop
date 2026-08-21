<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'
  import { listen } from '@tauri-apps/api/event'
  import { onMount } from 'svelte'
  import { t } from '../i18n'

  type RemoteStatus = {
    phase: 'off' | 'starting' | 'up' | 'error'
    url: string | null
    link: string | null
    error: string | null
    proxy_port: number | null
  }

  let status = $state<RemoteStatus | null>(null)
  let qrSvg = $state('')
  let copied = $state(false)
  let resetDone = $state(false)
  let busy = $state(false)

  onMount(() => {
    let unlisten: (() => void) | undefined
    invoke<RemoteStatus>('get_remote_status').then((s) => (status = s))
    listen<RemoteStatus>('remote-status', (e) => (status = e.payload)).then((u) => (unlisten = u))
    return () => unlisten?.()
  })

  // 链接变化（每次开启 token 都换新）时重新取二维码
  $effect(() => {
    const link = status?.link
    if (link) {
      invoke<string>('get_remote_qr')
        .then((svg) => (qrSvg = svg))
        .catch(() => (qrSvg = ''))
    } else {
      qrSvg = ''
    }
  })

  async function toggle() {
    if (busy || !status) return
    busy = true
    try {
      const cmd = status.phase === 'off' || status.phase === 'error' ? 'start_remote' : 'stop_remote'
      status = await invoke<RemoteStatus>(cmd)
    } finally {
      busy = false
    }
  }

  async function copy() {
    try {
      await invoke('copy_remote_link')
      copied = true
      setTimeout(() => (copied = false), 2000)
    } catch {
      // 链接未就绪时按钮本就禁用；竞态下静默即可（托盘有 toast 反馈的场景在 tray.rs）
    }
  }

  // 重置链接：token 原地轮换 + 掐断现有会话，端口不变；旧链接/旧设备即刻失效
  async function resetLink() {
    if (busy || !status) return
    if (!window.confirm(t('重置后当前链接与所有已连接的设备都会立即失效，确定重置？'))) return
    busy = true
    try {
      status = await invoke<RemoteStatus>('reset_remote_link')
      resetDone = true
      setTimeout(() => (resetDone = false), 2000)
    } catch {
      // 未开启时按钮本不显示；竞态下静默即可
    } finally {
      busy = false
    }
  }
</script>

<main>
  <header>
    <h1>{t('远程访问')}</h1>
  </header>

  {#if !status}
    <p class="tip">{t('加载中…')}</p>
  {:else}
    <section class="card pad">
      {#if status.phase === 'up' && status.link}
        <p class="tip center">{t('用手机扫码或复制链接打开')}</p>
        {#if qrSvg}
          <div class="qr">{@html qrSvg}</div>
        {/if}
        <p class="link">{status.link}</p>
        <div class="actions">
          <button class="primary" onclick={copy}>{copied ? t('已复制') : t('复制链接')}</button>
          <button onclick={resetLink} disabled={busy}>{resetDone ? t('已重置') : t('重置链接')}</button>
          <button onclick={toggle} disabled={busy}>{t('关闭远程访问')}</button>
        </div>
      {:else if status.phase === 'starting'}
        <p class="tip center">{t('正在开启远程访问…')}</p>
        <p class="tip center dim">{t('请确保手机与电脑连接同一网络')}</p>
        <div class="actions">
          <button onclick={toggle} disabled={busy}>{t('关闭远程访问')}</button>
        </div>
      {:else}
        <p class="tip center">{t('远程访问未开启')}</p>
        {#if status.phase === 'error' && status.error}
          <p class="errtext">{status.error}</p>
        {/if}
        <div class="actions">
          <button class="primary" onclick={toggle} disabled={busy}>{t('开启远程访问')}</button>
        </div>
      {/if}
    </section>
    <p class="tip warnline">{t('链接即凭据，请勿分享给他人')}</p>
    <p class="tip dim">{t('每次开启都会生成新链接，旧链接即刻失效')}</p>
    <p class="tip dim">{t('链接泄露时点“重置链接”立即吊销，端口不变')}</p>
    <p class="tip dim">{t('若手机无法访问，请检查 Windows 防火墙是否放行 DSHDesktop 与该端口')}</p>
    <p class="tip dim">{t('SSH 隧道模式下链接为 http://服务器地址:暴露端口，需服务器在线且允许远程转发')}</p>
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
  .card {
    background: var(--bg-raise);
    border: 1px solid var(--border);
    border-radius: 10px;
  }
  .pad {
    padding: 18px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 14px;
  }
  .tip {
    margin: 0;
    font-size: 12px;
    color: var(--text-3);
  }
  .center {
    text-align: center;
  }
  .dim {
    color: var(--text-4);
  }
  .warnline {
    text-align: center;
    color: var(--warn);
  }
  .errtext {
    margin: 0;
    font-size: 12px;
    color: var(--bad);
    text-align: center;
    word-break: break-all;
  }
  .qr {
    background: #fff;
    border-radius: 8px;
    padding: 10px;
    line-height: 0;
  }
  .link {
    margin: 0;
    font-size: 12px;
    color: var(--text-2);
    word-break: break-all;
    text-align: center;
    user-select: text;
  }
  .actions {
    display: flex;
    gap: 10px;
    justify-content: center;
  }
  button {
    border: none;
    border-radius: 8px;
    padding: 8px 18px;
    font-size: 13px;
    cursor: pointer;
    background: var(--bg-track);
    color: var(--text);
  }
  button.primary {
    background: var(--accent);
    color: #fff;
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
