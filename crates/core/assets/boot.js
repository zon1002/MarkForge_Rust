// 完整頁面（Viewer、檔案總管預覽）的啟動腳本：選項放在 <html data-mf-options>，
// 不用行內 <script>，頁面才能套用禁止行內腳本的 Content-Security-Policy。
(function () {
  'use strict';
  let opts = {};
  try { opts = JSON.parse(document.documentElement.dataset.mfOptions || '{}'); } catch (e) { /* 預設值 */ }
  MarkForgePreview.init(opts);
  window.mfReady = MarkForgePreview.enhance(document.getElementById('content'), opts);
})();
