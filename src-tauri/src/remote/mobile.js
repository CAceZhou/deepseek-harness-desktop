/*
 * DSHDesktop 远程访问移动端增强：
 * 1) 会话页"对话/轨迹"旁追加"信息"标签，点开展示回合统计面板
 *    （轮步/LLM 耗时/首 token/缓存/token 用量）。
 * 2) 输入卡片工具行（"+" 旁）注入回形针附件按钮，手机端从系统文件
 *    选择器传图片进草稿（上游只有拖拽/剪贴板两条入口，手机都没有）。
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

/*
 * 附件按钮：上游输入只有拖拽/剪贴板两条图片入口（onPaste → intakeImages
 * → createDraftImages → base64 → session.prompt），手机浏览器一条都没有。
 * 这里在输入卡片工具行（"+" 旁）注入一个样式克隆自 "+" 的回形针按钮，
 * 点击调起系统文件选择器，选完构造 DataTransfer 合成 paste 事件喂回上游
 * 自己的粘贴管线——类型/数量/体积校验与报错 toast 全部复用上游逻辑。
 *
 * 注意：上游 host（dsh-attachment admitEncodedImages，sharp 校验）只认
 * png/jpeg/webp/gif 四种位图，文档类型上游不支持，选择器因此只开图片。
 * 按钮与 file input 都是我们加的异物节点，input 放 body 下（React 树外），
 * 按钮插入 tools 行（与信息标签同款先例：只加不移 React 节点）；
 * React 重渲染抹掉按钮时由观察器按 DOM 缺席重挂。
 */
;(() => {
  try {
    const BTN_ATTR = 'data-dshmobile-attach'
    const narrow = () => matchMedia('(max-width: 700px)').matches
    const zh = () => (document.documentElement.lang || '').toLowerCase().startsWith('zh')
    // Feather 风格回形针（24 视窗描边图标，与 dsh 的 outline 图标同风）
    const PAPERCLIP_SVG =
      '<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m21.44 11.05-9.19 9.19a6 6 0 0 1-8.49-8.49l8.57-8.57A4 4 0 1 1 18 8.84l-8.59 8.57a2 2 0 0 1-2.83-2.83l8.49-8.48"/></svg>'

    // 共享一个 file input（body 下、React 树外）；点击时记录目标卡片。
    // 惰性创建：本脚本注入点在 </head> 前，执行时 body 可能还没出来。
    let picker = null
    let targetCard = null
    const ensurePicker = () => {
      if (picker) return picker
      if (!document.body) return null
      picker = document.createElement('input')
      picker.type = 'file'
      picker.accept = 'image/png,image/jpeg,image/webp,image/gif'
      picker.multiple = true
      picker.style.display = 'none'
      picker.addEventListener('change', () => {
        const files = Array.from(picker.files || [])
        picker.value = ''
        const card = targetCard
        targetCard = null
        if (files.length === 0 || !card || !card.isConnected) return
        const textarea = card.querySelector('textarea')
        if (!textarea) return
        try {
          const dt = new DataTransfer()
          for (const f of files) dt.items.add(f)
          textarea.dispatchEvent(
            new ClipboardEvent('paste', { clipboardData: dt, bubbles: true, cancelable: true }),
          )
        } catch {
          /* 静默 */
        }
      })
      document.body.appendChild(picker)
      return picker
    }

    const setup = (tools) => {
      const addBtn = tools.querySelector('button[class*="_add"]')
      const card = tools.closest('[class*="_card"]')
      if (!addBtn || !card || !card.querySelector('textarea')) return
      // "+" 可能包在 Tooltip 的 wrapper 里：锚定它在 tools 行的直接子代
      let anchor = addBtn
      while (anchor.parentElement && anchor.parentElement !== tools) anchor = anchor.parentElement

      const btn = document.createElement('button')
      btn.type = 'button'
      btn.className = addBtn.className // 克隆 "+" 的类名，28px 圆形同款
      btn.disabled = addBtn.disabled
      btn.setAttribute(BTN_ATTR, '')
      btn.setAttribute('aria-label', zh() ? '添加图片附件' : 'Add image attachment')
      btn.innerHTML = PAPERCLIP_SVG
      anchor.after(btn)

      // 与上游 keepFocus 一致：按下不抢输入框焦点
      btn.addEventListener('mousedown', (e) => e.preventDefault())
      btn.addEventListener('click', () => {
        if (btn.disabled) return
        const p = ensurePicker()
        if (!p) return
        targetCard = card
        p.value = ''
        p.click()
      })
    }

    const ensure = () => {
      // 离开窄断点（旋屏/拉窗）时摘除按钮，桌面形态保持原生
      if (!narrow()) {
        for (const b of document.querySelectorAll(`button[${BTN_ATTR}]`)) b.remove()
        return
      }
      for (const tools of document.querySelectorAll('div[class*="_tools"]')) {
        try {
          const existing = tools.querySelector(`button[${BTN_ATTR}]`)
          if (existing) {
            // 跟随 "+" 的禁用态（锁定/忙时上游 onPaste 也会拒收，双保险）
            const addBtn = tools.querySelector('button[class*="_add"]')
            if (addBtn) existing.disabled = addBtn.disabled
            continue
          }
          setup(tools)
        } catch {
          /* 静默 */
        }
      }
    }

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

