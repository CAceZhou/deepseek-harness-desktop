/*
 * DSHDesktop 远程访问移动端增强：会话页"对话/轨迹"旁追加"信息"标签，
 * 点开展示回合统计面板（轮步/LLM 耗时/首 token/缓存/token 用量）。
 *
 * 设计要点：
 * - 渐进增强：任何一步找不到目标节点就静默放弃——mobile.css 里统计行的
 *   两行换行样式是兜底，本脚本失效时信息仍在输入区下方。
 * - 统计节点不搬家，用 MutationObserver 克隆同步：React 对被移走的节点
 *   removeChild 会抛 NotFoundError（切会话卸载统计行时必崩）。
 * - 增强生效（标签已挂上）后给 <html> 打 data-dshmobile-enhanced，
 *   mobile.css 借此隐藏输入区下方的原统计行。
 * - 信息面板打开时只动视觉：原生标签的激活态被 CSS 降级，React 状态不受影响；
 *   点"对话/轨迹"（捕获期监听）退出信息页。
 */
(() => {
  try {
    const TAB_ATTR = 'data-dshmobile-tab'
    const PANEL_ATTR = 'data-dshmobile-info'
    const OPEN_ATTR = 'data-dshmobile-info-open'
    const ENHANCED_ATTR = 'data-dshmobile-enhanced'

    const narrow = () => matchMedia('(max-width: 700px)').matches
    const zh = () => (document.documentElement.lang || '').toLowerCase().startsWith('zh')
    const L = {
      info: () => (zh() ? '信息' : 'Info'),
      title: () => (zh() ? '回合统计' : 'Turn stats'),
      empty: () =>
        zh()
          ? '暂无回合统计——完成一轮对话后，这里会显示耗时与 token 用量。'
          : 'No turn stats yet. Finish a turn and timing/token usage shows up here.',
    }

    // 统计行 = 输入区栈内、输入卡片之外、直接子代带 ≥2 个分隔符的 root
    const findStatsRoot = (chatRoot) => {
      for (const r of chatRoot.querySelectorAll('[class*="_composerStack"] [class*="_root"]')) {
        if (r.closest('[class*="_card"]')) continue
        let seps = 0
        for (const c of r.children) {
          if ((c.className || '').includes('_sep')) seps++
        }
        if (seps >= 2) return r
      }
      return null
    }

    const setup = (tablist) => {
      const chatRoot = tablist.closest('[class*="_root"]')
      const anyTab = tablist.querySelector('[role="tab"]')
      const header = tablist.closest('header')
      if (!chatRoot || !anyTab || !header) return

      // 信息标签：类名克隆自原生标签（摘掉激活态类），视觉与"对话/轨迹"一致
      const btn = document.createElement('button')
      btn.type = 'button'
      btn.setAttribute('role', 'tab')
      btn.setAttribute(TAB_ATTR, '')
      btn.className = anyTab.className.replace(/\S*_tabActive\S*/g, '').trim()
      btn.textContent = L.info()
      tablist.appendChild(btn)

      // 信息面板：绝对定位盖在会话区上（顶边 = 头部高），输入条 z7 在其上仍可输入
      const panel = document.createElement('div')
      panel.setAttribute(PANEL_ATTR, '')
      const card = document.createElement('div')
      card.setAttribute('data-dshmobile-info-card', '')
      const h2 = document.createElement('h2')
      h2.textContent = L.title()
      const body = document.createElement('div')
      body.setAttribute('data-dshmobile-info-body', '')
      card.appendChild(h2)
      card.appendChild(body)
      panel.appendChild(card)
      chatRoot.appendChild(panel)

      const syncStats = () => {
        const stats = findStatsRoot(chatRoot)
        let clone = body.querySelector('[data-dshmobile-stats]')
        if (!stats) {
          if (clone) clone.remove()
          if (!body.querySelector('[data-dshmobile-empty]')) {
            const empty = document.createElement('p')
            empty.setAttribute('data-dshmobile-empty', '')
            empty.textContent = L.empty()
            body.appendChild(empty)
          }
          return
        }
        if (!clone) {
          body.innerHTML = ''
          clone = document.createElement('div')
          clone.setAttribute('data-dshmobile-stats', '')
          body.appendChild(clone)
        }
        if (clone.className !== stats.className) clone.className = stats.className
        if (clone.innerHTML !== stats.innerHTML) clone.innerHTML = stats.innerHTML
      }

      btn.addEventListener('click', () => {
        panel.style.top = `${header.offsetHeight}px`
        chatRoot.setAttribute(OPEN_ATTR, '')
        tablist.setAttribute('data-dshmobile-open', '')
        btn.setAttribute('data-active', '')
        syncStats()
      })
      // 捕获期：点原生"对话/轨迹"退出信息页（React 自己的标签切换不受影响）
      tablist.addEventListener(
        'click',
        (e) => {
          const t = e.target.closest('[role="tab"]')
          if (!t || t.hasAttribute(TAB_ATTR)) return
          chatRoot.removeAttribute(OPEN_ATTR)
          tablist.removeAttribute('data-dshmobile-open')
          btn.removeAttribute('data-active')
        },
        true,
      )

      // 统计行文本随回合推进更新（秒级 tick 与回合结束），面板开着时跟随同步
      new MutationObserver(() => {
        if (chatRoot.hasAttribute(OPEN_ATTR)) syncStats()
      }).observe(chatRoot, { subtree: true, childList: true, characterData: true })

      // 标签挂上了：增强生效，原统计行改由信息页呈现（CSS 隐藏输入区下方的原行）。
      // 跟随断点：旋屏/拉窗离开 ≤700px 时摘掉增强标记，原统计行恢复显示，
      // 否则宽屏下标签被 CSS 隐藏、原行也被隐藏，统计就无处可见了。
      const mq = matchMedia('(max-width: 700px)')
      const applyEnhanced = () => {
        if (mq.matches) document.documentElement.setAttribute(ENHANCED_ATTR, '')
        else document.documentElement.removeAttribute(ENHANCED_ATTR)
      }
      mq.addEventListener('change', applyEnhanced)
      applyEnhanced()
    }

    const ensure = () => {
      if (!narrow()) return
      const tablist = document.querySelector('header [role="tablist"]')
      if (!tablist || tablist.querySelector(`[${TAB_ATTR}]`)) return
      setup(tablist)
    }

    // 视图随导航挂载/卸载：监听文档子树，标签栏出现且没挂过就挂
    new MutationObserver(() => {
      try {
        ensure()
      } catch {
        /* 静默 */
      }
    }).observe(document.documentElement, { subtree: true, childList: true })
    ensure()
  } catch {
    /* 静默：增强失败不影响页面 */
  }
})()
