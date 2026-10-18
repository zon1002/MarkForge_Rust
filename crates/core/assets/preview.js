// MarkForge 預覽頁面腳本：語法上色、Mermaid、圖表縮放平移、相對路徑圖片、同步捲動對位。
// Editor 的預覽窗格、Viewer、檔案總管預覽與匯出的 HTML 共用這一份。
(function () {
  'use strict';

  const BLOCK_ATOMIC = new Set(['PRE', 'TABLE']);
  // comrak 也會替行內元素（del、em、code…）加 data-sourcepos，錨點只取區塊元素
  const BLOCK_TAGS = new Set(['P', 'H1', 'H2', 'H3', 'H4', 'H5', 'H6', 'LI', 'PRE', 'TABLE', 'BLOCKQUOTE',
    'HR', 'DT', 'DD', 'DIV', 'UL', 'OL', 'DL', 'SECTION']);
  const BLOCK_SELECTOR = Array.from(BLOCK_TAGS).map(function (t) { return t.toLowerCase() + '[data-sourcepos]'; }).join(',');

  // ---------- 初始化 ----------

  function init(opts) {
    if (window.mermaid) {
      mermaid.initialize({ startOnLoad: false, theme: opts && opts.dark ? 'dark' : 'default', securityLevel: 'strict' });
    }
  }

  // ---------- 內容處理 ----------

  // 相對路徑與 Windows 絕對路徑的圖片改由 App 提供（docBase／absBase 為 null 時不改寫，
  // 例如匯出的 HTML 放在 .md 旁邊，相對路徑本來就能用）。
  function resolveImages(root, opts) {
    if (!opts || !opts.docBase) return;
    root.querySelectorAll('img[src], video[src], source[src]').forEach(function (el) {
      let src = el.getAttribute('src');
      if (!src || el.dataset.mfResolved) return;
      let raw = src;
      try { raw = decodeURIComponent(src); } catch (e) { /* 保留原字串 */ }
      if (/^file:\/\/\//i.test(raw)) raw = raw.substring(8);
      if (/^[a-z]:[\\/]/i.test(raw)) {
        if (opts.absBase) el.setAttribute('src', opts.absBase + encodeURI(raw.replace(/\\/g, '/')));
      } else if (!/^(?:[a-z][a-z0-9+.-]*:|\/|#)/i.test(src)) {
        el.setAttribute('src', opts.docBase + src.replace(/^\.\//, ''));
      }
      el.dataset.mfResolved = '1';
    });
  }

  function highlight(root) {
    if (!window.hljs) return;
    root.querySelectorAll('pre code').forEach(function (el) {
      if (el.closest('pre.mermaid')) return;
      try { hljs.highlightElement(el); } catch (e) { /* 不認得的語言就維持原樣 */ }
    });
  }

  async function renderMermaid(root) {
    if (!window.mermaid) return;
    const nodes = Array.from(root.querySelectorAll('pre.mermaid:not([data-processed])'));
    if (nodes.length === 0) return;
    try {
      await mermaid.run({ nodes: nodes, suppressErrors: true });
    } catch (e) {
      console.error('Mermaid render failed', e);
    }
    nodes.forEach(setupPanZoom);
  }

  // 依序做完所有增強；回傳的 Promise 在 Mermaid 畫完後才完成（PDF 匯出要等它）。
  async function enhance(root, opts) {
    resolveImages(root, opts);
    highlight(root);
    await renderMermaid(root);
  }

  // ---------- Mermaid 縮放／平移 ----------
  // 每張圖獨立：滾輪以游標為中心縮放（0.2～12 倍，不需按 Ctrl）、左鍵拖曳平移、按兩下還原。

  let drag = null;

  function applyTransform(svg, st) {
    svg.style.transformOrigin = '0 0';
    svg.style.transform = 'translate(' + st.tx + 'px, ' + st.ty + 'px) scale(' + st.scale + ')';
  }

  function setupPanZoom(pre) {
    const svg = pre.querySelector('svg');
    if (!svg || pre.dataset.mfPanZoom) return;
    pre.dataset.mfPanZoom = '1';
    const st = { scale: 1, tx: 0, ty: 0 };
    applyTransform(svg, st);

    pre.addEventListener('wheel', function (e) {
      e.preventDefault();
      const rect = pre.getBoundingClientRect();
      const cx = e.clientX - rect.left;
      const cy = e.clientY - rect.top;
      const next = Math.min(12, Math.max(0.2, st.scale * (e.deltaY < 0 ? 1.15 : 1 / 1.15)));
      st.tx = cx - (cx - st.tx) * (next / st.scale);
      st.ty = cy - (cy - st.ty) * (next / st.scale);
      st.scale = next;
      applyTransform(svg, st);
    }, { passive: false });

    pre.addEventListener('mousedown', function (e) {
      if (e.button !== 0) return;
      drag = { svg: svg, st: st, pre: pre, x: e.clientX, y: e.clientY, tx: st.tx, ty: st.ty };
      pre.style.cursor = 'grabbing';
      e.preventDefault();
    });

    pre.addEventListener('dblclick', function () {
      st.scale = 1; st.tx = 0; st.ty = 0;
      applyTransform(svg, st);
    });
  }

  window.addEventListener('mousemove', function (e) {
    if (!drag) return;
    drag.st.tx = drag.tx + (e.clientX - drag.x);
    drag.st.ty = drag.ty + (e.clientY - drag.y);
    applyTransform(drag.svg, drag.st);
  });
  window.addEventListener('mouseup', function () {
    if (!drag) return;
    drag.pre.style.cursor = 'grab';
    drag = null;
  });

  // ---------- 同步捲動對位 ----------
  // 以 comrak 的 data-sourcepos（每個區塊元素的起訖行）建立「原始碼行號 ↔ 像素位置」錨點表。
  // 只取最內層的區塊（段落、清單項、標題……），表格與程式碼／Mermaid 區塊視為一整塊。
  // 區塊內部依比例內插；原始碼幾行的 Mermaid 可能畫成整頁高的圖，整塊處理才不會漂移。

  function parsePos(el) {
    const m = /^(\d+):\d+-(\d+):\d+$/.exec(el.getAttribute('data-sourcepos') || '');
    return m ? { start: parseInt(m[1], 10), end: parseInt(m[2], 10) } : null;
  }

  function anchors(root, scroller) {
    const base = scroller === window || scroller === document.documentElement
      ? -window.scrollY
      : scroller.getBoundingClientRect().top - scroller.scrollTop;
    const list = [];
    root.querySelectorAll(BLOCK_SELECTOR).forEach(function (el) {
      const outer = el.parentElement && el.parentElement.closest('pre, table');
      if (outer && root.contains(outer)) return;                       // 表格／程式碼內部
      if (!BLOCK_ATOMIC.has(el.tagName) && el.querySelector(BLOCK_SELECTOR)) return; // 非最內層區塊
      const pos = parsePos(el);
      if (!pos) return;
      const rect = el.getBoundingClientRect();
      if (rect.height === 0 && rect.width === 0) return;
      list.push({ start: pos.start, end: pos.end, top: rect.top - base, height: rect.height, atomic: BLOCK_ATOMIC.has(el.tagName) });
    });
    list.sort(function (a, b) { return a.start - b.start || a.top - b.top; });
    return list;
  }

  // 原始碼行號（1 起算）→ 捲動位置
  function lineToOffset(root, scroller, line) {
    const list = anchors(root, scroller);
    if (list.length === 0) return 0;
    let a = null;
    for (const item of list) { if (item.start <= line) a = item; else break; }
    if (!a) return 0;
    if (line <= a.end) {
      if (a.atomic) return a.top;
      const span = a.end - a.start + 1;
      return a.top + a.height * ((line - a.start) / span);
    }
    // 落在兩個區塊之間的空白行：在 a 的底與下一塊的頂之間內插
    const next = list[list.indexOf(a) + 1];
    if (!next) return a.top + a.height;
    const t = (line - a.end) / Math.max(1, next.start - a.end);
    return a.top + a.height + t * (next.top - (a.top + a.height));
  }

  // 捲動位置 → 原始碼行號（可為小數）
  function offsetToLine(root, scroller, offset) {
    const list = anchors(root, scroller).slice().sort(function (x, y) { return x.top - y.top; });
    if (list.length === 0) return 1;
    let a = null;
    for (const item of list) { if (item.top <= offset) a = item; else break; }
    if (!a) return 1;
    if (offset <= a.top + a.height) {
      if (a.atomic) return a.start;
      const t = a.height > 0 ? (offset - a.top) / a.height : 0;
      return a.start + t * (a.end - a.start + 1);
    }
    const next = list[list.indexOf(a) + 1];
    if (!next) return a.end;
    const gap = Math.max(1, next.top - (a.top + a.height));
    return a.end + ((offset - a.top - a.height) / gap) * Math.max(0, next.start - a.end);
  }

  window.MarkForgePreview = {
    init: init,
    enhance: enhance,
    resolveImages: resolveImages,
    lineToOffset: lineToOffset,
    offsetToLine: offsetToLine,
  };
})();
