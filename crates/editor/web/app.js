// MarkForge Editor 前端：CodeMirror 6 編輯區、預覽、工具列、狀態列、彈出選單與對話框。
// 與 Rust 的溝通：window.ipc.postMessage(JSON)；Rust 以 app.xxx(...) 回呼。
(function () {
  'use strict';

  const {
    EditorState, EditorSelection, Compartment, EditorView, keymap, lineNumbers, highlightActiveLine,
    highlightActiveLineGutter, drawSelection, dropCursor, highlightSpecialChars, rectangularSelection,
    crosshairCursor, defaultKeymap, history, historyKeymap, indentWithTab, undo, redo, selectAll, markdown,
    markdownLanguage, syntaxHighlighting, HighlightStyle, indentUnit, search, searchKeymap,
    highlightSelectionMatches, openSearchPanel, tags,
  } = window.CM;

  const $ = (sel) => document.querySelector(sel);
  const ipc = (msg) => window.ipc.postMessage(JSON.stringify(msg));
  // 前端錯誤送到 Rust 的診斷記錄（MARKFORGE_DEBUG_LOG）
  window.addEventListener('error', (e) => ipc({ cmd: 'log', message: e.message + ' @' + e.lineno }));
  window.addEventListener('unhandledrejection', (e) => ipc({ cmd: 'log', message: 'unhandled: ' + e.reason }));
  const NEW_FILE_LABEL = '新檔案（尚未儲存）';

  const state = {
    settings: null,
    fonts: [],
    path: null,
    savedDoc: null,
    dirty: false,
    renderSeq: 0,
    renderWaiters: new Map(),
    plainPasteNext: false,
  };

  // ============================== 編輯區 ==============================

  const themeSlot = new Compartment();
  const fontSlot = new Compartment();

  function highlightStyle() {
    const muted = { color: 'var(--ed-muted)' };
    return HighlightStyle.define([
      { tag: tags.heading1, fontSize: '1.6em', fontWeight: '700' },
      { tag: tags.heading2, fontSize: '1.4em', fontWeight: '700' },
      { tag: tags.heading3, fontSize: '1.25em', fontWeight: '700' },
      { tag: tags.heading4, fontSize: '1.12em', fontWeight: '700' },
      { tag: [tags.heading5, tags.heading6], fontWeight: '700' },
      { tag: tags.strong, fontWeight: '700' },
      { tag: tags.emphasis, fontStyle: 'italic' },
      { tag: tags.strikethrough, textDecoration: 'line-through', color: 'var(--ed-muted)' },
      { tag: [tags.link, tags.url], color: 'var(--ed-link)', textDecoration: 'underline' },
      { tag: tags.monospace, backgroundColor: 'var(--ed-code-bg)', fontFamily: 'Consolas, "Cascadia Code", monospace' },
      { tag: tags.quote, fontStyle: 'italic', color: 'var(--ed-muted)' },
      { tag: tags.list, color: 'var(--ed-list)' },
      { tag: [tags.processingInstruction, tags.contentSeparator, tags.meta], ...muted },
    ]);
  }

  function editorTheme(dark) {
    return EditorView.theme({
      '&': { backgroundColor: 'var(--ed-bg)', color: 'var(--ed-text)', height: '100%' },
      '.cm-content': { caretColor: dark ? '#ffffff' : '#000000', padding: '10px 0 40vh' },
      '.cm-gutters': { backgroundColor: 'var(--ed-bg)', color: 'var(--ed-gutter)', border: 'none' },
      '.cm-activeLine': { backgroundColor: 'var(--ed-active-line)' },
      '.cm-activeLineGutter': { backgroundColor: 'var(--ed-active-line)' },
      '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection': { backgroundColor: 'var(--ed-selection) !important' },
      '.cm-cursor': { borderLeftColor: dark ? '#ffffff' : '#000000' },
      '.cm-line': { padding: '0 12px 0 6px' },
    }, { dark: dark });
  }

  function fontTheme(s) {
    const family = '"' + s.editorFont.replace(/"/g, '') + '", Consolas, "Microsoft JhengHei", monospace';
    return EditorView.theme({
      '&': { fontSize: s.editorFontSize + 'px' },
      '.cm-scroller': { fontFamily: family },
    });
  }

  const view = new EditorView({
    parent: $('#editor-pane'),
    state: makeState(''),
  });

  function makeState(text) {
    return EditorState.create({
      doc: text,
      extensions: [
        lineNumbers(),
        highlightActiveLineGutter(),
        highlightSpecialChars(),
        history(),
        drawSelection(),
        dropCursor(),
        rectangularSelection(),
        crosshairCursor(),
        highlightActiveLine(),
        highlightSelectionMatches(),
        EditorState.allowMultipleSelections.of(true),
        EditorView.lineWrapping,
        indentUnit.of('    '),
        search({ top: true }),
        // markdown() 內含 Enter 自動接續清單／引用、Backspace 刪除清單符號
        markdown({ base: markdownLanguage }),
        syntaxHighlighting(highlightStyle()),
        keymap.of([...defaultKeymap, ...historyKeymap, ...searchKeymap, indentWithTab]),
        themeSlot.of(editorTheme(state.settings ? state.settings.dark : true)),
        fontSlot.of(state.settings ? fontTheme(state.settings) : []),
        EditorView.updateListener.of(onUpdate),
        EditorView.domEventHandlers({
          paste: onPaste,
          drop: onDrop,
          contextmenu: onContextMenu,
        }),
      ],
    });
  }

  function onUpdate(update) {
    if (update.docChanged) {
      updateDirty();
      scheduleRender();
    }
    if (update.docChanged || update.selectionSet) updateStatus();
  }

  function updateDirty() {
    const dirty = !state.savedDoc || !view.state.doc.eq(state.savedDoc);
    if (dirty !== state.dirty) {
      state.dirty = dirty;
      ipc({ cmd: 'dirty', dirty: dirty });
    }
  }

  // ---------- 貼上：由 Rust 讀取原生剪貼簿並轉換 ----------
  // 瀏覽器的 clipboardData 只有 HTML 片段（Word 的樣式表不在其中），也拿不到 Excel 格式與檔案路徑。

  function onPaste(e) {
    e.preventDefault();
    ipc({ cmd: 'paste', plain: state.plainPasteNext });
    state.plainPasteNext = false;
    return true;
  }

  // ---------- 拖入檔案：把 File 物件交給 Rust 取真實路徑 ----------

  function hasFiles(e) {
    return e.dataTransfer && Array.from(e.dataTransfer.types || []).includes('Files');
  }

  function postDrop(e, target) {
    const files = e.dataTransfer.files;
    if (!files || files.length === 0) return false;
    e.preventDefault();
    // 訊息必須是字串：帶附加物件時，物件型別的訊息 WebView2 不會送達（實測）
    const msg = JSON.stringify({ type: 'drop', x: e.clientX, y: e.clientY, target: target });
    try {
      window.chrome.webview.postMessageWithAdditionalObjects(msg, files);
    } catch (err) {
      ipc({ cmd: 'log', message: 'postMessageWithAdditionalObjects failed: ' + err });
    }
    return true;
  }

  function onDrop(e) {
    return hasFiles(e) ? postDrop(e, 'editor') : false;
  }

  document.addEventListener('dragover', (e) => {
    if (hasFiles(e)) { e.preventDefault(); e.dataTransfer.dropEffect = 'copy'; }
  });
  document.addEventListener('drop', (e) => {
    if (hasFiles(e) && !e.defaultPrevented) postDrop(e, 'other');
    else e.preventDefault();
  });

  // ============================== 插入與格式 ==============================

  /** 以一個復原步驟取代選取範圍（或插入在指定座標）；區塊內容前後補空行，避免黏到相鄰段落。 */
  function insertText(text, block, at) {
    let from, to;
    if (at && at.x != null) {
      const pos = view.posAtCoords({ x: at.x, y: at.y }, false);
      from = to = pos == null ? view.state.selection.main.head : pos;
    } else {
      ({ from, to } = view.state.selection.main);
    }
    const doc = view.state.doc;
    if (block) {
      const before = doc.sliceString(Math.max(0, from - 2), from);
      const after = doc.sliceString(to, Math.min(doc.length, to + 2));
      if (from > 0 && !before.endsWith('\n\n')) text = (before.endsWith('\n') ? '\n' : '\n\n') + text;
      // 區塊之後一定留一個空行：表格後緊接的一行（例如接著貼上的圖片）會被 GFM 當成表格的下一列
      if (!after.startsWith('\n\n')) text += after.startsWith('\n') ? '\n' : '\n\n';
    }
    view.dispatch({
      changes: { from: from, to: to, insert: text },
      selection: { anchor: from + text.length },
      scrollIntoView: true,
      userEvent: 'input.paste',
    });
    view.focus();
  }

  function selectedLines() {
    const { from, to } = view.state.selection.main;
    const doc = view.state.doc;
    const first = doc.lineAt(from).number;
    let last = doc.lineAt(to).number;
    if (last > first && to === doc.line(last).from) last--; // 選取停在下一行行首時不算那一行
    const lines = [];
    for (let n = first; n <= last; n++) lines.push(doc.line(n));
    return lines;
  }

  function caretToLineEnd(lineNumber) {
    const line = view.state.doc.line(Math.min(lineNumber, view.state.doc.lines));
    view.dispatch({ selection: { anchor: line.to } });
    view.focus();
  }

  /** **粗體**、*斜體*、~~刪除線~~、`程式碼`：已包覆就移除，否則包上。 */
  function toggleWrap(marker) {
    const { from, to } = view.state.selection.main;
    const doc = view.state.doc;
    const selected = doc.sliceString(from, to);
    const n = marker.length;
    if (selected.length >= n * 2 && selected.startsWith(marker) && selected.endsWith(marker)) {
      const inner = selected.slice(n, selected.length - n);
      view.dispatch({ changes: { from, to, insert: inner }, selection: { anchor: from, head: from + inner.length } });
    } else if (from >= n && doc.sliceString(from - n, from) === marker && doc.sliceString(to, to + n) === marker) {
      view.dispatch({
        changes: [{ from: from - n, to: from }, { from: to, to: to + n }],
        selection: { anchor: from - n, head: to - n },
      });
    } else {
      const replacement = marker + selected + marker;
      view.dispatch({
        changes: { from, to, insert: replacement },
        selection: selected.length === 0 ? { anchor: from + n } : { anchor: from, head: from + replacement.length },
      });
    }
    view.focus();
  }

  function heading(level) {
    const line = view.state.doc.lineAt(view.state.selection.main.head);
    const m = /^(#{1,6})(\s+)/.exec(line.text);
    const prefix = '#'.repeat(level) + ' ';
    let change;
    if (m && m[1].length === level) change = { from: line.from, to: line.from + m[0].length, insert: '' };
    else if (m) change = { from: line.from, to: line.from + m[0].length, insert: prefix };
    else change = { from: line.from, insert: prefix };
    view.dispatch({ changes: change });
    view.focus();
  }

  /** 每行行首（縮排之後）切換前綴：全部都有就移除，否則補上。 */
  function togglePrefix(prefix) {
    const lines = selectedLines();
    const all = lines.every((l) => l.text.replace(/^[ \t]*/, '').startsWith(prefix));
    const changes = [];
    for (const l of lines) {
      const indent = /^[ \t]*/.exec(l.text)[0].length;
      const has = l.text.slice(indent).startsWith(prefix);
      if (all && has) changes.push({ from: l.from + indent, to: l.from + indent + prefix.length });
      else if (!all && !has) changes.push({ from: l.from + indent, insert: prefix });
    }
    view.dispatch({ changes });
    caretToLineEnd(lines[lines.length - 1].number);
  }

  function orderedList() {
    const lines = selectedLines();
    const re = /^([ \t]*)(\d+)([.)])(\s+)/;
    const all = lines.every((l) => re.test(l.text));
    const changes = lines.map((l, i) => {
      if (all) {
        const m = re.exec(l.text);
        return { from: l.from, to: l.from + m[0].length, insert: m[1] };
      }
      return { from: l.from, insert: (i + 1) + '. ' };
    });
    view.dispatch({ changes });
    caretToLineEnd(lines[lines.length - 1].number);
  }

  /** 設定（或加上）工作清單標記 - [x]；已經有標記就只換括號內的字元。 */
  function taskMarker(symbol) {
    const lines = selectedLines();
    const re = /^([ \t]*)[-*+]\s+\[(.)\]\s+/;
    const changes = lines.map((l) => {
      const m = re.exec(l.text);
      if (m) {
        const at = l.from + m[0].indexOf('[') + 1;
        return { from: at, to: at + m[2].length, insert: symbol };
      }
      const indent = /^[ \t]*/.exec(l.text)[0].length;
      return { from: l.from + indent, insert: '- [' + symbol + '] ' };
    });
    view.dispatch({ changes });
    caretToLineEnd(lines[lines.length - 1].number);
  }

  function codeBlock() {
    const { from, to } = view.state.selection.main;
    const selected = view.state.sliceDoc(from, to);
    const block = selected.length > 0 ? '```\n' + selected + '\n```' : '```\n\n```';
    view.dispatch({
      changes: { from, to, insert: block },
      selection: { anchor: selected.length > 0 ? from + block.length : from + 4 },
    });
    view.focus();
  }

  /** [文字](網址)／![替代文字](路徑)：有選取時選取文字成為標籤並選取網址，否則選取標籤占位字。 */
  function linkTemplate(open, label, url) {
    const { from, to } = view.state.selection.main;
    const selected = view.state.sliceDoc(from, to);
    const text = selected.length > 0 ? selected : label;
    const insert = open + text + '](' + url + ')';
    const sel = selected.length > 0
      ? { anchor: from + open.length + text.length + 2, head: from + open.length + text.length + 2 + url.length }
      : { anchor: from + open.length, head: from + open.length + text.length };
    view.dispatch({ changes: { from, to, insert }, selection: sel });
    view.focus();
  }

  function insertBlockTemplate(text, leading) {
    const pos = view.state.selection.main.head;
    const needsNewline = pos > 0 && view.state.sliceDoc(pos - 1, pos) !== '\n';
    const insert = (needsNewline ? leading : '') + text;
    view.dispatch({ changes: { from: pos, insert }, selection: { anchor: pos + insert.length } });
    view.focus();
  }

  const TABLE_TEMPLATE = '| 欄位 1 | 欄位 2 | 欄位 3 |\n| --- | --- | --- |\n| 內容 | 內容 | 內容 |\n| 內容 | 內容 | 內容 |\n';

  // ============================== 指令 ==============================

  function toRust(name, arg) {
    ipc({ cmd: 'command', name: name, arg: arg == null ? null : String(arg), text: view.state.doc.toString() });
  }

  function execEdit(command) {
    view.focus();
    document.execCommand(command);
  }

  const commands = {
    'file.new': () => toRust('file.new'),
    'file.open': () => toRust('file.open'),
    'file.openPath': (p) => toRust('file.openPath', p),
    'file.save': () => toRust('file.save'),
    'file.saveAs': () => toRust('file.saveAs'),
    'file.exportHtml': () => toRust('file.exportHtml'),
    // PDF 印的是預覽：先確定預覽是最新內容、Mermaid 已畫完。WebView2 列印不含背景色，
    // 深色主題直接印會變成白紙上的淺灰字，所以先切成淺色再印（app.pdfDone 時還原）。
    'file.exportPdf': async () => {
      if (state.settings.dark) setPageTheme(false);
      await renderNow();
      toRust('file.exportPdf');
    },
    'app.exit': () => toRust('app.exit'),
    'edit.undo': () => { undo(view); view.focus(); },
    'edit.redo': () => { redo(view); view.focus(); },
    'edit.cut': () => execEdit('cut'),
    'edit.copy': () => execEdit('copy'),
    'edit.paste': () => ipc({ cmd: 'paste', plain: false }),
    'edit.pastePlain': () => ipc({ cmd: 'paste', plain: true }),
    'edit.selectAll': () => { selectAll(view); view.focus(); },
    'edit.find': () => { openSearchPanel(view); },
    'fmt.bold': () => toggleWrap('**'),
    'fmt.italic': () => toggleWrap('*'),
    'fmt.strike': () => toggleWrap('~~'),
    'fmt.inlineCode': () => toggleWrap('`'),
    'fmt.heading': (n) => heading(Number(n)),
    'fmt.quote': () => togglePrefix('> '),
    'fmt.ul': () => togglePrefix('- '),
    'fmt.ol': orderedList,
    'fmt.task': (mark) => (mark ? taskMarker(mark) : togglePrefix('- [ ] ')),
    'fmt.codeBlock': codeBlock,
    'fmt.link': () => linkTemplate('[', '連結文字', 'https://'),
    'fmt.imageTemplate': () => linkTemplate('![', '替代文字', 'image-url'),
    'fmt.table': () => insertBlockTemplate(TABLE_TEMPLATE, '\n'),
    'fmt.hr': () => insertBlockTemplate('---\n\n', '\n\n'),
    'insert.image': () => toRust('insert.image'),
    'insert.table': () => toRust('insert.table'),
    'insert.mermaid': () => toRust('insert.mermaid'),
    'insert.attachment': () => toRust('insert.attachment'),
    'view.toggleTheme': () => toRust('view.toggleTheme'),
    'view.toggleEditor': () => toRust('view.toggleEditor'),
    'view.togglePreview': () => toRust('view.togglePreview'),
    'view.editorFont': () => fontDialog('editor'),
    'view.previewFont': () => fontDialog('preview'),
    'view.widthCustom': widthDialog,
    'help.about': () => toRust('help.about'),
  };

  function runCommand(name, arg) {
    hidePopup();
    if (commands[name]) return commands[name](arg);
    if (name.startsWith('view.width:')) return toRust(name);
    console.warn('unknown command', name);
  }

  // 快捷鍵（選單只顯示，實際在這裡處理）
  const SHORTCUTS = {
    'Ctrl+n': 'file.new', 'Ctrl+o': 'file.open', 'Ctrl+s': 'file.save', 'Ctrl+Shift+s': 'file.saveAs',
    'Ctrl+b': 'fmt.bold', 'Ctrl+i': 'fmt.italic', 'Ctrl+k': 'fmt.link',
  };
  window.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') { hidePopup(); closeModal(); }
    if (!$('#modal-backdrop').hidden) return;
    // 原生選單的助憶鍵（檔案 F、編輯 E、插入 I、檢視 V、說明 H）與 F10
    if (e.altKey && !e.ctrlKey && !e.shiftKey && /^[feivh]$/i.test(e.key)) {
      e.preventDefault();
      ipc({ cmd: 'menuKey', key: e.key.toLowerCase() });
      return;
    }
    if (e.key === 'F10' && !e.ctrlKey && !e.altKey) {
      e.preventDefault();
      ipc({ cmd: 'menuKey', key: '' });
      return;
    }
    const key = (e.ctrlKey ? 'Ctrl+' : '') + (e.shiftKey ? 'Shift+' : '') + (e.altKey ? 'Alt+' : '') + e.key.toLowerCase();
    if (key === 'Ctrl+Shift+v') { state.plainPasteNext = true; return; } // 接著觸發的 paste 事件改貼純文字
    const name = SHORTCUTS[key];
    if (name) { e.preventDefault(); e.stopPropagation(); runCommand(name); }
  }, true);

  // ============================== 預覽 ==============================

  const preview = $('#preview');
  const previewPane = $('#preview-pane');
  let renderTimer = null;

  function scheduleRender() {
    clearTimeout(renderTimer);
    renderTimer = setTimeout(requestRender, 250);
  }

  function requestRender() {
    clearTimeout(renderTimer);
    const id = ++state.renderSeq;
    ipc({ cmd: 'render', id: id, text: view.state.doc.toString() });
    return id;
  }

  /** 立即渲染並等到 Mermaid 畫完（另存 PDF 前使用）。 */
  function renderNow() {
    const id = requestRender();
    return new Promise((resolve) => state.renderWaiters.set(id, resolve));
  }

  async function rendered(id, html) {
    if (id !== state.renderSeq) return; // 已經有更新的內容在路上
    preview.innerHTML = html;
    await MarkForgePreview.enhance(preview, { docBase: '/doc/', absBase: '/abs/' });
    syncPreviewFromEditor();
    const waiter = state.renderWaiters.get(id);
    if (waiter) { state.renderWaiters.delete(id); waiter(); }
  }

  // 預覽中的連結：網址交給預設瀏覽器，# 錨點在預覽內捲動，其他（相對路徑）不處理
  preview.addEventListener('click', (e) => {
    const a = e.target.closest('a[href]');
    if (!a) return;
    e.preventDefault();
    const href = a.getAttribute('href');
    if (href.startsWith('#')) {
      const target = preview.querySelector('[id="' + CSS.escape(decodeURIComponent(href.slice(1))) + '"]');
      if (target) previewPane.scrollTop += target.getBoundingClientRect().top - previewPane.getBoundingClientRect().top;
    } else if (/^(https?:|mailto:)/i.test(href)) {
      ipc({ cmd: 'openExternal', url: a.href });
    }
  });

  // ---------- 同步捲動 ----------
  // 以原始碼行號為共同座標：編輯區頂端行號 ↔ 預覽中對應區塊的位置（見 preview.js）。

  let syncing = null;

  function editorTopLine() {
    const top = view.scrollDOM.scrollTop - view.documentPadding.top;
    const block = view.lineBlockAtHeight(Math.max(0, top));
    const line = view.state.doc.lineAt(block.from).number;
    const frac = block.height > 0 ? Math.min(1, Math.max(0, (top - block.top) / block.height)) : 0;
    return line + frac;
  }

  function syncPreviewFromEditor() {
    if ($('#main').classList.contains('hide-preview')) return;
    syncing = 'preview';
    previewPane.scrollTop = MarkForgePreview.lineToOffset(preview, previewPane, editorTopLine());
    requestAnimationFrame(() => { syncing = null; });
  }

  function syncEditorFromPreview() {
    const line = MarkForgePreview.offsetToLine(preview, previewPane, previewPane.scrollTop);
    const n = Math.min(Math.max(1, Math.floor(line)), view.state.doc.lines);
    const block = view.lineBlockAt(view.state.doc.line(n).from);
    syncing = 'editor';
    view.scrollDOM.scrollTop = block.top + (line - n) * block.height + view.documentPadding.top;
    requestAnimationFrame(() => { syncing = null; });
  }

  let editorScrollTick = false;
  view.scrollDOM.addEventListener('scroll', () => {
    if (syncing === 'editor' || editorScrollTick) return;
    editorScrollTick = true;
    requestAnimationFrame(() => { editorScrollTick = false; syncPreviewFromEditor(); });
  }, { passive: true });

  let previewScrollTick = false;
  previewPane.addEventListener('scroll', () => {
    if (syncing === 'preview' || previewScrollTick) return;
    previewScrollTick = true;
    requestAnimationFrame(() => { previewScrollTick = false; syncEditorFromPreview(); });
  }, { passive: true });

  // ============================== 狀態列 ==============================

  function updateStatus() {
    const doc = view.state.doc;
    const head = view.state.selection.main.head;
    const line = doc.lineAt(head);
    $('#st-pos').textContent = '行 ' + line.number + '，列 ' + (head - line.from + 1);
    const text = doc.toString();
    const words = (text.match(/[\p{L}\p{N}]+/gu) || []).length;
    $('#st-count').textContent = words + ' 字 / ' + text.length + ' 字元';
    $('#st-path').textContent = state.path || NEW_FILE_LABEL;
    $('#st-path').title = state.path || '';
  }

  // ============================== 工具列 ==============================

  function icon(name) {
    return (window.ICONS && window.ICONS[name]) || '';
  }

  function makeButton(iconName, label, title, onClick) {
    const b = document.createElement('button');
    b.className = 'tb-btn';
    b.title = title || label;
    b.innerHTML = icon(iconName) + (label ? '<span>' + label + '</span>' : '');
    b.addEventListener('click', onClick);
    b.addEventListener('mousedown', (e) => e.preventDefault()); // 保持編輯區焦點與選取
    return b;
  }

  function dropdownButton(iconName, label, title, items, mainAction) {
    if (mainAction) {
      const wrap = document.createElement('span');
      wrap.className = 'tb-split';
      const main = makeButton(iconName, label, title, mainAction);
      const arrow = makeButton('', '', title, (e) => showPopup(items, arrow, e));
      arrow.innerHTML = '<span class="caret">▼</span>';
      wrap.append(main, arrow);
      return wrap;
    }
    const b = makeButton(iconName, label, title, (e) => showPopup(items, b, e));
    b.insertAdjacentHTML('beforeend', '<span class="caret">▼</span>');
    return b;
  }

  function buildToolbar() {
    const tb = $('#toolbar');
    const sep = () => { const s = document.createElement('span'); s.className = 'tb-sep'; return s; };
    const cmd = (name, arg) => () => runCommand(name, arg);
    tb.append(
      makeButton('fmt-bold', '粗體', '粗體 (Ctrl+B)', cmd('fmt.bold')),
      makeButton('fmt-italic', '斜體', '斜體 (Ctrl+I)', cmd('fmt.italic')),
      makeButton('fmt-strike', '刪除線', '刪除線', cmd('fmt.strike')),
      sep(),
      dropdownButton('fmt-heading', '標題', '標題 1～6', [1, 2, 3, 4, 5, 6].map((n) => ({ label: '標題 ' + n, run: cmd('fmt.heading', n) }))),
      sep(),
      makeButton('fmt-quote', '引用', '引用區塊', cmd('fmt.quote')),
      makeButton('fmt-ul', '項目清單', '項目符號清單', cmd('fmt.ul')),
      makeButton('fmt-ol', '編號清單', '編號清單', cmd('fmt.ol')),
      dropdownButton('fmt-task', '工作清單', '工作清單（核取方塊）— 箭頭可選擇打勾／打叉／三角形／驚嘆號', [
        { label: '打勾', icon: 'task-done', run: cmd('fmt.task', 'x') },
        { label: '打叉', icon: 'task-cross', run: cmd('fmt.task', '✗') },
        { label: '三角形', icon: 'task-triangle', run: cmd('fmt.task', '▲') },
        { label: '驚嘆號', icon: 'task-exclaim', run: cmd('fmt.task', '!') },
      ], cmd('fmt.task')),
      sep(),
      makeButton('fmt-code-inline', '行內程式碼', '行內程式碼', cmd('fmt.inlineCode')),
      makeButton('fmt-code-block', '程式碼區塊', '程式碼區塊', cmd('fmt.codeBlock')),
      sep(),
      makeButton('fmt-link', '連結', '插入連結 (Ctrl+K)', cmd('fmt.link')),
      dropdownButton('insert-image', '圖片', '選擇圖片檔案插入（存檔時自動複製到 .md 所在資料夾）；也可直接拖入或貼上圖片', [
        { label: '選擇圖片檔案...', icon: 'file-open', run: cmd('insert.image') },
        { label: '插入圖片語法範本', icon: 'insert-image', run: cmd('fmt.imageTemplate') },
      ], cmd('insert.image')),
      dropdownButton('insert-table', '表格', '插入表格範本；也可直接拖入 Excel／CSV 檔案，或貼上複製的儲存格', [
        { label: '插入表格範本', icon: 'insert-table', run: cmd('fmt.table') },
        { label: '從 Excel／CSV／ODS 檔案匯入...', icon: 'file-open', run: cmd('insert.table') },
      ], cmd('fmt.table')),
      makeButton('fmt-hr', '分隔線', '插入水平分隔線', cmd('fmt.hr')),
      sep(),
      makeButton('edit-undo', '復原', '復原 (Ctrl+Z)', cmd('edit.undo')),
      makeButton('edit-redo', '重做', '重做 (Ctrl+Y)', cmd('edit.redo')),
    );
  }

  // ============================== 彈出選單 ==============================

  const popup = $('#popup');

  function showPopup(items, anchor, event) {
    if (event) event.stopPropagation();
    popup.innerHTML = '';
    for (const it of items) {
      if (it.sep) { const s = document.createElement('div'); s.className = 'pm-sep'; popup.append(s); continue; }
      const row = document.createElement('div');
      row.className = 'pm-item' + (it.disabled ? ' disabled' : '');
      const iconHtml = it.icon ? icon(it.icon) : '';
      row.innerHTML = '<span class="pm-icon">' + iconHtml + '</span><span>' + it.label + '</span>' +
        (it.key ? '<span class="pm-key">' + it.key + '</span>' : '');
      row.addEventListener('mousedown', (e) => e.preventDefault());
      row.addEventListener('click', () => { hidePopup(); it.run(); });
      popup.append(row);
    }
    popup.hidden = false;
    let x, y;
    if (anchor) {
      const r = anchor.getBoundingClientRect();
      x = r.left; y = r.bottom + 2;
    } else {
      x = event.clientX; y = event.clientY;
    }
    const pr = popup.getBoundingClientRect();
    popup.style.left = Math.max(0, Math.min(x, window.innerWidth - pr.width - 4)) + 'px';
    popup.style.top = Math.max(0, Math.min(y, window.innerHeight - pr.height - 4)) + 'px';
  }

  function hidePopup() { popup.hidden = true; }
  document.addEventListener('mousedown', (e) => { if (!popup.contains(e.target)) hidePopup(); });
  window.addEventListener('blur', hidePopup);

  function onContextMenu(e) {
    e.preventDefault();
    const hasSelection = !view.state.selection.main.empty;
    showPopup([
      { label: '復原', key: 'Ctrl+Z', icon: 'edit-undo', run: () => runCommand('edit.undo') },
      { label: '重做', key: 'Ctrl+Y', icon: 'edit-redo', run: () => runCommand('edit.redo') },
      { sep: true },
      { label: '剪下', key: 'Ctrl+X', icon: 'edit-cut', disabled: !hasSelection, run: () => runCommand('edit.cut') },
      { label: '複製', key: 'Ctrl+C', icon: 'edit-copy', disabled: !hasSelection, run: () => runCommand('edit.copy') },
      { label: '貼上', key: 'Ctrl+V', icon: 'edit-paste', run: () => runCommand('edit.paste') },
      { label: '貼上為純文字', key: 'Ctrl+Shift+V', icon: 'edit-paste-plain', run: () => runCommand('edit.pastePlain') },
      { sep: true },
      { label: '全選', key: 'Ctrl+A', icon: 'edit-select-all', run: () => runCommand('edit.selectAll') },
    ], null, e);
    return true;
  }
  // 預覽區的右鍵：只提供複製
  previewPane.addEventListener('contextmenu', (e) => {
    e.preventDefault();
    const hasSelection = !!String(window.getSelection() || '');
    showPopup([{ label: '複製', key: 'Ctrl+C', icon: 'edit-copy', disabled: !hasSelection, run: () => document.execCommand('copy') }], null, e);
  });

  // ============================== 對話框 ==============================

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

  function closeModal() {
    if (backdrop.hidden) return;
    backdrop.hidden = true;
    modal.innerHTML = '';
    view.focus();
  }

  function escapeHtml(s) {
    return String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
  }

  function fontDialog(target) {
    const s = state.settings;
    const family = target === 'editor' ? s.editorFont : s.previewFont;
    const size = target === 'editor' ? s.editorFontSize : s.previewFontSize;
    const options = state.fonts.map((f) => '<option' + (f === family ? ' selected' : '') + '>' + escapeHtml(f) + '</option>').join('');
    openModal(
      '<h2>' + (target === 'editor' ? '編輯器字型' : '預覽字型') + '</h2>' +
      '<label>篩選</label><input id="font-filter" placeholder="輸入字型名稱">' +
      '<label>字型</label><select id="font-family" size="8">' + options + '</select>' +
      '<label>大小（px）</label><input id="font-size" type="number" min="8" max="48" step="0.5" value="' + size + '">' +
      '<div class="sample" id="font-sample">MarkForge Markdown 編輯器 AaBbCc 123</div>',
      () => {
        const f = $('#font-family').value;
        const sz = parseFloat($('#font-size').value);
        if (!f || !(sz >= 8 && sz <= 48)) return false;
        ipc({ cmd: 'settings', patch: target === 'editor' ? { editorFont: f, editorFontSize: sz } : { previewFont: f, previewFontSize: sz } });
      });
    const sel = $('#font-family');
    const sample = () => {
      $('#font-sample').style.fontFamily = '"' + sel.value + '"';
      $('#font-sample').style.fontSize = $('#font-size').value + 'px';
    };
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
    const current = state.settings.previewWidth > 0 ? state.settings.previewWidth : 860;
    openModal(
      '<h2>自訂預覽寬度</h2><label>寬度（px）</label>' +
      '<input id="width-px" type="number" min="320" max="4000" step="10" value="' + current + '">' +
      '<div class="hint">320～4000 px；內容置中顯示。此設定也套用到匯出的 HTML。</div>',
      () => {
        const w = Math.round(parseFloat($('#width-px').value));
        if (!(w >= 320 && w <= 4000)) return false;
        ipc({ cmd: 'settings', patch: { previewWidth: w } });
      });
  }

  // ============================== 設定與版面 ==============================

  function setPageTheme(dark) {
    document.documentElement.dataset.theme = dark ? 'dark' : 'light';
    $('#hljs-css').href = '/assets/' + (dark ? 'github-dark.min.css' : 'github.min.css');
    MarkForgePreview.init({ dark: dark });
  }

  function applySettings(s) {
    const themeChanged = !state.settings || state.settings.dark !== s.dark;
    state.settings = s;
    setPageTheme(s.dark);
    view.dispatch({ effects: [themeSlot.reconfigure(editorTheme(s.dark)), fontSlot.reconfigure(fontTheme(s))] });

    preview.style.setProperty('--mf-font-family', '"' + s.previewFont.replace(/"/g, '') + '", "Segoe UI", "Microsoft JhengHei", sans-serif');
    preview.style.setProperty('--mf-font-size', s.previewFontSize + 'px');
    preview.style.setProperty('--mf-width', s.previewWidth > 0 ? s.previewWidth + 'px' : 'none');

    const main = $('#main');
    main.classList.toggle('hide-editor', !s.showEditor);
    main.classList.toggle('hide-preview', !s.showPreview);
    $('#editor-pane').style.flexBasis = (s.split * 100) + '%';

    if (themeChanged) requestRender(); // Mermaid 要以新主題重畫
  }

  // 分隔線拖曳
  (function () {
    const splitter = $('#splitter');
    splitter.addEventListener('mousedown', (e) => {
      e.preventDefault();
      splitter.classList.add('dragging');
      const main = $('#main').getBoundingClientRect();
      const move = (ev) => {
        const ratio = Math.min(0.85, Math.max(0.15, (ev.clientX - main.left) / main.width));
        $('#editor-pane').style.flexBasis = (ratio * 100) + '%';
        state.settings.split = ratio;
      };
      const up = () => {
        splitter.classList.remove('dragging');
        window.removeEventListener('mousemove', move);
        window.removeEventListener('mouseup', up);
        ipc({ cmd: 'settings', patch: { split: state.settings.split } });
      };
      window.addEventListener('mousemove', move);
      window.addEventListener('mouseup', up);
    });
  })();

  // ============================== Rust → 前端 ==============================

  window.app = {
    init(payload) {
      state.fonts = payload.fonts || [];
      applySettings(payload.settings);
    },
    applySettings: applySettings,
    command: runCommand,
    rendered: rendered,

    loadDocument(doc) {
      view.setState(makeState(doc.text));
      state.path = doc.path;
      state.savedDoc = view.state.doc;
      state.dirty = false;
      view.scrollDOM.scrollTop = 0;
      previewPane.scrollTop = 0;
      updateStatus();
      requestRender();
      view.focus();
    },

    saved(info) {
      if (info.text != null) {
        // 附件改名時 Rust 會改寫連結：整份替換但保留游標與捲動位置
        const head = Math.min(view.state.selection.main.head, info.text.length);
        const top = view.scrollDOM.scrollTop;
        view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: info.text }, selection: { anchor: head } });
        view.scrollDOM.scrollTop = top;
      }
      state.path = info.path;
      state.savedDoc = view.state.doc;
      state.dirty = false;
      updateStatus();
      requestRender(); // 圖片現在從文件資料夾提供
    },

    insert(s) {
      insertText(s.text, s.block, s.x != null ? { x: s.x, y: s.y } : null);
    },

    /** 另存 PDF 結束（完成、取消或失敗）：還原深色預覽。 */
    pdfDone() {
      if (state.settings.dark) {
        setPageTheme(true);
        requestRender();
      }
    },
  };

  buildToolbar();
  updateStatus();
  ipc({ cmd: 'ready' });
})();
