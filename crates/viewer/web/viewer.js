// MarkForge Viewer 前端：工具列、內容顯示、字型／寬度對話框。與 Rust 以 window.ipc 溝通。
(function () {
  'use strict';

  const $ = (sel) => document.querySelector(sel);
  const ipc = (msg) => window.ipc.postMessage(JSON.stringify(msg));
  window.addEventListener('error', (e) => ipc({ cmd: 'log', message: e.message + ' @' + e.lineno }));

  const content = $('#content');
  const pane = $('#view-pane');
  const state = { settings: null, fonts: [], path: null, printPending: false };

  const WIDTHS = [[860, '860px（預設）'], [1000, '1000px'], [1200, '1200px'], [1400, '1400px'], [0, '不限制寬度']];

  // ============================== 顯示 ==============================

  async function show(doc) {
    const top = pane.scrollTop;
    state.path = doc.path;
    content.innerHTML = doc.html;
    await MarkForgePreview.enhance(content, { docBase: '/doc/', absBase: '/abs/' });
    pane.scrollTop = doc.keepScroll ? top : 0;
    if (state.printPending) {
      state.printPending = false;
      ipc({ cmd: 'exportPdf' });
    }
  }

  // 連結：網址交給預設瀏覽器，# 錨點在頁內捲動
  content.addEventListener('click', (e) => {
    const a = e.target.closest('a[href]');
    if (!a) return;
    e.preventDefault();
    const href = a.getAttribute('href');
    if (href.startsWith('#')) {
      const target = content.querySelector('[id="' + CSS.escape(decodeURIComponent(href.slice(1))) + '"]');
      if (target) pane.scrollTop += target.getBoundingClientRect().top - pane.getBoundingClientRect().top;
    } else if (/^(https?:|mailto:)/i.test(href)) {
      ipc({ cmd: 'openExternal', url: a.href });
    }
  });

  // 拖入 .md 檔案開啟：File 物件交給 Rust 取路徑（訊息必須是字串，見 markforge-shell::webview2）
  document.addEventListener('dragover', (e) => { e.preventDefault(); e.dataTransfer.dropEffect = 'copy'; });
  document.addEventListener('drop', (e) => {
    e.preventDefault();
    const files = e.dataTransfer && e.dataTransfer.files;
    if (files && files.length) {
      window.chrome.webview.postMessageWithAdditionalObjects(JSON.stringify({ type: 'drop', x: e.clientX, y: e.clientY, target: 'viewer' }), files);
    }
  });

  window.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') { hidePopup(); closeModal(); return; }
    if (!$('#modal-backdrop').hidden) return;
    if (e.ctrlKey && !e.shiftKey && e.key.toLowerCase() === 'o') { e.preventDefault(); ipc({ cmd: 'open' }); }
    else if (e.key === 'F5') { e.preventDefault(); ipc({ cmd: 'reload' }); }
  }, true);

  // ============================== 工具列 ==============================

  function icon(name) { return (window.ICONS && window.ICONS[name]) || ''; }

  function button(iconName, label, title, onClick) {
    const b = document.createElement('button');
    b.className = 'tb-btn';
    b.title = title;
    b.innerHTML = icon(iconName) + '<span>' + label + '</span>';
    b.addEventListener('click', onClick);
    return b;
  }

  function buildToolbar() {
    const widthBtn = button('layout-split', '寬度', '調整閱讀版面寬度', (e) => showPopup(widthItems(), widthBtn, e));
    widthBtn.insertAdjacentHTML('beforeend', '<span class="caret">▼</span>');
    $('#toolbar').append(
      button('folder2-open', '開啟...', '開啟 Markdown 檔案 (Ctrl+O)', () => ipc({ cmd: 'open' })),
      button('arrow-clockwise', '重新整理', '重新載入 (F5)', () => ipc({ cmd: 'reload' })),
      button('moon-stars', '深色/淺色', '切換主題', () => ipc({ cmd: 'settings', patch: { dark: !state.settings.dark } })),
      button('fonts', '字型...', '選擇預覽字型與大小', fontDialog),
      widthBtn,
      button('filetype-pdf', '另存PDF...', '將目前內容另存為 PDF', exportPdf),
    );
  }

  function widthItems() {
    const w = state.settings.width;
    const preset = WIDTHS.some(([px]) => px === w);
    return WIDTHS.map(([px, label]) => ({ label: label, checked: px === w, run: () => ipc({ cmd: 'settings', patch: { width: px } }) }))
      .concat([{ sep: true }, { label: preset ? '自訂寬度...' : '自訂寬度...（目前 ' + w + 'px）', checked: !preset, run: widthDialog }]);
  }

  // PDF：WebView2 列印不含背景色，深色主題先切成淺色再印，完成後（app.pdfDone）還原
  function exportPdf() {
    if (!state.settings.dark || !state.path) { ipc({ cmd: 'exportPdf' }); return; }
    // 重新載入讓 Mermaid 以淺色重畫；show() 畫完後才送出匯出
    state.printPending = true;
    setPageTheme(false);
    ipc({ cmd: 'reload' });
  }

  // ============================== 彈出選單與對話框 ==============================

  const popup = $('#popup');
  function showPopup(items, anchor, event) {
    event.stopPropagation();
    popup.innerHTML = '';
    for (const it of items) {
      if (it.sep) { const s = document.createElement('div'); s.className = 'pm-sep'; popup.append(s); continue; }
      const row = document.createElement('div');
      row.className = 'pm-item';
      row.innerHTML = '<span class="pm-icon">' + (it.checked ? '✓' : '') + '</span><span>' + it.label + '</span>';
      row.addEventListener('click', () => { hidePopup(); it.run(); });
      popup.append(row);
    }
    popup.hidden = false;
    const r = anchor.getBoundingClientRect();
    popup.style.left = r.left + 'px';
    popup.style.top = (r.bottom + 2) + 'px';
  }
  function hidePopup() { popup.hidden = true; }
  document.addEventListener('mousedown', (e) => { if (!popup.contains(e.target)) hidePopup(); });

  const backdrop = $('#modal-backdrop');
  const modal = $('#modal');
  function openModal(html, onOk) {
    modal.innerHTML = html + '<div class="buttons"><button class="primary" data-ok>確定</button><button data-cancel>取消</button></div>';
    backdrop.hidden = false;
    modal.querySelector('[data-cancel]').onclick = closeModal;
    modal.querySelector('[data-ok]').onclick = () => { if (onOk() !== false) closeModal(); };
    modal.addEventListener('keydown', (e) => { if (e.key === 'Enter' && e.target.tagName !== 'SELECT') modal.querySelector('[data-ok]').click(); });
    const first = modal.querySelector('input, select');
    if (first) first.focus();
  }
  function closeModal() { backdrop.hidden = true; modal.innerHTML = ''; }
  function escapeHtml(s) { return String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c])); }

  function fontDialog() {
    const s = state.settings;
    const options = state.fonts.map((f) => '<option' + (f === s.font ? ' selected' : '') + '>' + escapeHtml(f) + '</option>').join('');
    openModal(
      '<h2>預覽字型</h2><label>篩選</label><input id="font-filter" placeholder="輸入字型名稱">' +
      '<label>字型</label><select id="font-family" size="8">' + options + '</select>' +
      '<label>大小（px）</label><input id="font-size" type="number" min="8" max="48" step="0.5" value="' + s.fontSize + '">' +
      '<div class="sample" id="font-sample">MarkForge Markdown 檢視器 AaBbCc 123</div>',
      () => {
        const f = $('#font-family').value, sz = parseFloat($('#font-size').value);
        if (!f || !(sz >= 8 && sz <= 48)) return false;
        ipc({ cmd: 'settings', patch: { font: f, fontSize: sz } });
      });
    const sel = $('#font-family');
    const sample = () => { $('#font-sample').style.fontFamily = '"' + sel.value + '"'; $('#font-sample').style.fontSize = $('#font-size').value + 'px'; };
    sel.addEventListener('change', sample);
    $('#font-size').addEventListener('input', sample);
    $('#font-filter').addEventListener('input', (e) => {
      const q = e.target.value.toLowerCase();
      for (const o of sel.options) o.hidden = q && !o.value.toLowerCase().includes(q);
    });
    if (sel.selectedOptions[0]) sel.selectedOptions[0].scrollIntoView({ block: 'center' });
    sample();
  }

  function widthDialog() {
    const current = state.settings.width > 0 ? state.settings.width : 860;
    openModal(
      '<h2>自訂閱讀寬度</h2><label>寬度（px）</label><input id="width-px" type="number" min="320" max="4000" step="10" value="' + current + '">' +
      '<div class="hint">320～4000 px；內容置中顯示。</div>',
      () => {
        const w = Math.round(parseFloat($('#width-px').value));
        if (!(w >= 320 && w <= 4000)) return false;
        ipc({ cmd: 'settings', patch: { width: w } });
      });
  }

  // ============================== 設定 ==============================

  function setPageTheme(dark) {
    document.documentElement.dataset.theme = dark ? 'dark' : 'light';
    $('#hljs-css').href = '/assets/' + (dark ? 'github-dark.min.css' : 'github.min.css');
    MarkForgePreview.init({ dark: dark });
  }

  function applySettings(s) {
    const themeChanged = state.settings && state.settings.dark !== s.dark;
    state.settings = s;
    setPageTheme(s.dark);
    content.style.setProperty('--mf-font-family', '"' + s.font.replace(/"/g, '') + '", "Segoe UI", "Microsoft JhengHei", sans-serif');
    content.style.setProperty('--mf-font-size', s.fontSize + 'px');
    content.style.setProperty('--mf-width', s.width > 0 ? s.width + 'px' : 'none');
    if (themeChanged && state.path) ipc({ cmd: 'reload' }); // Mermaid 以新主題重畫
  }

  window.app = {
    init(payload) { state.fonts = payload.fonts || []; applySettings(payload.settings); },
    applySettings: applySettings,
    show: show,
    empty() { content.innerHTML = '<p class="empty-hint">尚未開啟任何檔案。可拖放 .md 檔案到此視窗，或使用「開啟」按鈕。</p>'; },
    pdfDone() {
      if (state.settings.dark) { setPageTheme(true); ipc({ cmd: 'reload' }); }
    },
  };

  buildToolbar();
  ipc({ cmd: 'ready' });
})();
