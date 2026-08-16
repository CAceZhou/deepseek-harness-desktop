<script lang="ts">
  import { listen } from '@tauri-apps/api/event'
  import { invoke } from '@tauri-apps/api/core'
  import { onMount, onDestroy } from 'svelte'
  import { t } from '../i18n'

  type ProgressPayload = { stage: string; message: string; percent: number | null }

  // 等待就绪阶段前端缓动的上限：后端只在真正就绪时才报 100（与 progress.rs 一致）
  const CEILING = 95

  let firstLaunch = $state(false)
  let stage = $state('runtime')
  let message = $state(t('正在准备运行时…'))
  let floor = $state(0)      // 后端给定的进度下限（只增不减）
  let displayed = $state(0)  // 实际展示的百分比（含缓动）
  let unlistenProgress: (() => void) | undefined

  let isError = $derived(stage === 'error')

  // 首启进度条的阶段清单；currentStep 之前的全部视为已完成（语言跟随 dsh）
  const steps = ['准备运行时', '启动 dsh 服务', '等待服务就绪', '打开界面']
  let currentStep = $derived(
    stage === 'ready'
      ? steps.length
      : stage === 'starting'
        ? displayed >= floor + 10
          ? 2
          : 1
        : 0,
  )

  onMount(async () => {
    // 事件可能早于 listen 注册而丢失，主动查询引导错误与首启标记
    const bootError = await invoke<string | null>('get_bootstrap_error')
    firstLaunch = await invoke<boolean>('is_first_launch').catch(() => false)
    if (bootError) {
      stage = 'error'
      message = bootError
    }
    unlistenProgress = await listen<ProgressPayload>('dsh-progress', (e) => {
      const p = e.payload
      stage = p.stage
      message = p.message
      if (p.percent != null) {
        floor = Math.max(floor, p.percent)
        displayed = Math.max(displayed, floor)
      }
    })
  })

  // starting 阶段缓动：向 CEILING 渐近，视觉上持续前进但永不触顶；
  // 就绪事件到达时 percent=100 直接把 displayed 拉满
  $effect(() => {
    if (firstLaunch && stage === 'starting' && !isError) {
      const tm = setInterval(() => {
        displayed += (CEILING - displayed) * 0.045
        if (CEILING - displayed < 0.5) displayed = CEILING
      }, 200)
      return () => clearInterval(tm)
    }
  })

  onDestroy(() => {
    unlistenProgress?.()
  })
</script>

<main>
  <svg class="logo" width="72" height="72" viewBox="0 0 256 256" aria-hidden="true">
    <rect x="16" y="16" width="224" height="224" rx="48" fill="#1565c0" />
    <rect x="64" y="72" width="128" height="24" rx="12" fill="#fff" />
    <rect x="64" y="116" width="128" height="24" rx="12" fill="#fff" />
    <rect x="64" y="160" width="88" height="24" rx="12" fill="#fff" />
  </svg>
  <h1>DSHDesktop</h1>

  {#if isError}
    <p class="error">{message}</p>
  {:else if firstLaunch}
    <p>{message}</p>
    <p class="hint">{t('首次启动需要部署运行时，可能要花几分钟，请耐心等待')}</p>
    <div class="pct">{Math.round(displayed)}%</div>
    <div class="track">
      <div class="determinate" style="width: {Math.min(displayed, 100)}%"></div>
    </div>
    <ul class="steps">
      {#each steps as label, i}
        <li class:done={i < currentStep} class:active={i === currentStep}>
          <span class="mark">{i < currentStep ? '✓' : i === currentStep ? '●' : '○'}</span>
          {t(label)}
        </li>
      {/each}
    </ul>
  {:else}
    <p>{message}</p>
    {#if stage !== 'ready'}
      <div class="track narrow"><div class="slide"></div></div>
    {/if}
  {/if}
</main>

<style>
  main {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    gap: 12px;
    background: var(--bg);
  }
  h1 {
    font-size: 22px;
    font-weight: 600;
    letter-spacing: 0.5px;
    margin: 0;
    color: var(--text);
  }
  p {
    margin: 0;
    color: var(--text-2);
    font-size: 14px;
  }
  p.error {
    color: var(--bad);
    max-width: 70%;
    text-align: center;
    line-height: 1.5;
  }
  .hint {
    font-size: 12px;
    color: var(--text-3);
  }
  .pct {
    font-size: 28px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    color: var(--text);
  }
  .track {
    width: 260px;
    height: 4px;
    border-radius: 2px;
    background: var(--border);
    overflow: hidden;
  }
  .track.narrow {
    width: 220px;
  }
  .determinate {
    height: 100%;
    border-radius: 2px;
    background: var(--accent);
    transition: width 0.25s ease-out;
  }
  .slide {
    height: 100%;
    width: 40%;
    border-radius: 2px;
    background: var(--accent);
    animation: slide 1.2s ease-in-out infinite;
  }
  @keyframes slide {
    0% { transform: translateX(-100%); }
    100% { transform: translateX(320%); }
  }
  .steps {
    list-style: none;
    margin: 10px 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: 13px;
    color: var(--text-3);
  }
  .steps li {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .steps li.active {
    color: var(--text);
  }
  .steps li.done {
    color: var(--ok-strong);
  }
  .steps .mark {
    display: inline-block;
    width: 14px;
    text-align: center;
  }
</style>
