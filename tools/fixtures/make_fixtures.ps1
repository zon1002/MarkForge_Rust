# 重新產生 crates/core/tests/fixtures 的真實樣本（需 Microsoft Word、LibreOffice、Python + openpyxl）。
#   word_clipboard.html     Word「全選 → 複製」後剪貼簿 CF_HTML 的原始位元組（含標頭、可能混有 ANSI 的 <head>）
#   libreoffice_writer.html LibreOffice Writer 匯出的 HTML（與其剪貼簿 HTML 同一種方言）
#   book.xlsx / book.ods    兩個工作表 + 一個空白工作表，含日期、| 字元、多行儲存格
#   data_big5.csv           Big5 編碼、含引號與逗號的 CSV
#   flow.mmd                Mermaid 圖表
# 用法：powershell -STA -ExecutionPolicy Bypass -File tools\fixtures\make_fixtures.ps1
$ErrorActionPreference = 'Stop'
$out = Resolve-Path (Join-Path $PSScriptRoot '..\..\crates\core\tests\fixtures')
# 工作資料夾放在 target\ 底下（不納入版本控制）。放在 %TEMP% 時，無畫面的 Word 曾在 SaveAs2 卡住。
$work = Join-Path $PSScriptRoot ('..\..\target\fixtures-work-' + [Guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory $work | Out-Null

Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class RawClipboard {
  [DllImport("user32.dll", SetLastError=true)] static extern bool OpenClipboard(IntPtr h);
  [DllImport("user32.dll")] static extern bool CloseClipboard();
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern uint RegisterClipboardFormat(string name);
  [DllImport("user32.dll")] static extern IntPtr GetClipboardData(uint fmt);
  [DllImport("kernel32.dll")] static extern IntPtr GlobalLock(IntPtr h);
  [DllImport("kernel32.dll")] static extern bool GlobalUnlock(IntPtr h);
  [DllImport("kernel32.dll")] static extern UIntPtr GlobalSize(IntPtr h);
  public static byte[] Get(string format) {
    if (!OpenClipboard(IntPtr.Zero)) throw new Exception("OpenClipboard failed");
    try {
      IntPtr h = GetClipboardData(RegisterClipboardFormat(format));
      if (h == IntPtr.Zero) return null;
      IntPtr p = GlobalLock(h);
      int size = (int)GlobalSize(h);
      byte[] data = new byte[size];
      Marshal.Copy(p, data, 0, size);
      GlobalUnlock(h);
      return data;
    } finally { CloseClipboard(); }
  }
}
"@

# ---------- Word 文件 ----------
$word = New-Object -ComObject Word.Application
$word.Visible = $false
$word.DisplayAlerts = 0
try {
    $doc = $word.Documents.Add()
    $sel = $word.Selection
    function Para([string]$text, [int]$style, [double]$size = 0, [bool]$bold = $false) {
        $sel.Style = $style
        if ($size -gt 0) { $sel.Font.Size = $size }
        if ($bold) { $sel.Font.Bold = 1 }
        $sel.TypeText($text)
        $sel.TypeParagraph()
        $sel.Style = -1
        $sel.Font.Reset()
    }
    Para '專案年度報告' -63                       # 標題（Title）
    Para '第一章 概述' -2                          # 標題 1
    $sel.TypeText('這是一段一般內文，包含 ')
    $sel.Font.Bold = 1; $sel.TypeText('粗體文字'); $sel.Font.Bold = 0
    $sel.TypeText(' 與 ')
    $sel.Font.Italic = 1; $sel.TypeText('斜體文字'); $sel.Font.Italic = 0
    $sel.TypeText('，以及 snake_case 與 2*3 的特殊字元。這一行故意寫得比較長一些，確認 Word 在輸出 HTML 時自動換行不會在中文字之間插入多餘的空白字元。')
    $sel.TypeParagraph()
    Para '手動放大的標題文字' -1 22
    Para '第二節 細項' -3                          # 標題 2
    Para '稍微放大的小標題' -1 14 $true
    $sel.Range.ListFormat.ApplyBulletDefault()
    $sel.TypeText('第一個項目'); $sel.TypeParagraph()
    $sel.TypeText('第二個項目'); $sel.TypeParagraph()
    $sel.Range.ListFormat.RemoveNumbers()
    $sel.Range.ListFormat.ApplyNumberDefault()
    $sel.TypeText('步驟一'); $sel.TypeParagraph()
    $sel.TypeText('步驟二'); $sel.TypeParagraph()
    $sel.Range.ListFormat.RemoveNumbers()
    Para '以下為表格：' -1
    $table = $doc.Tables.Add($sel.Range, 3, 3)
    $data = @(@('品項', '數量', '單價'), @('蘋果', '10', '25.5'), @('香蕉|芭樂', '200', '8'))
    for ($r = 0; $r -lt 3; $r++) { for ($c = 0; $c -lt 3; $c++) { $table.Cell($r + 1, $c + 1).Range.Text = $data[$r][$c] } }
    $sel.EndKey(6) | Out-Null
    Para '結尾段落。' -1

    $doc.Content.Copy()
    $bytes = [RawClipboard]::Get('HTML Format')
    [IO.File]::WriteAllBytes((Join-Path $out 'word_clipboard.html'), $bytes)
    # 不另存 .docx：Office 更新後，無畫面的 Word 對這份文件執行 SaveAs2 會卡住（簡單文件則正常）
    $doc.Close(0)
}
finally {
    $word.Quit()
    [Runtime.InteropServices.Marshal]::ReleaseComObject($word) | Out-Null
}

# ---------- 試算表、CSV、Mermaid ----------
$py = @'
import sys, datetime, openpyxl
out = sys.argv[1]
wb = openpyxl.Workbook()
ws = wb.active; ws.title = "銷售"
ws.append(["日期", "品項", "數量", "金額"])
ws.append([datetime.date(2026, 9, 1), "蘋果", 10, 255.5])
ws.append([datetime.date(2026, 9, 2), "香蕉|芭樂", 200, 1600])
ws.append([None, "多行\n儲存格", 3, None])
ws2 = wb.create_sheet("庫存")
ws2.append(["倉庫", "數量"]); ws2.append(["台北", 5])
wb.create_sheet("空白")
wb.save(out + "/book.xlsx")
open(out + "/data_big5.csv", "w", encoding="cp950", newline="").write('名稱,數值,備註\n甲,1,"含,逗號"\n乙,2,"含""引號"""\n')
open(out + "/flow.mmd", "w", encoding="utf-8", newline="\n").write("graph TD\n  A[開始] --> B{判斷}\n  B -->|是| C[結束]\n")
'@
$pyFile = Join-Path $work 'make.py'
[IO.File]::WriteAllText($pyFile, $py, (New-Object Text.UTF8Encoding $false))
python $pyFile $out

# ---------- LibreOffice：Writer HTML 與 ODS ----------
# 建立一份 Writer 文件（標題樣式、手動放大字體、清單、表格），存成 .odt 後再匯出 HTML，
# 得到 LibreOffice 自己的 HTML 方言（與從 Writer 複製時剪貼簿的內容相同）。
# 用獨立的暫存使用者設定，避免與已在執行的 LibreOffice 共用同一個程序。
$writerSource = @'
<html><head><meta charset="utf-8"></head><body>
<h1>專案年度報告</h1>
<p>這是一段一般內文，包含 <b>粗體文字</b> 與 <i>斜體文字</i>。</p>
<p><font style="font-size: 22pt">手動放大的標題文字</font></p>
<h2>第二節 細項</h2>
<p><b><font style="font-size: 14pt">稍微放大的小標題</font></b></p>
<ul><li><p>第一個項目</p></li><li><p>第二個項目</p><ul><li><p>巢狀子項目</p></li></ul></li></ul>
<ol><li><p>步驟一</p></li><li><p>步驟二</p></li></ol>
<table border="1"><tr><td><p>品項</p></td><td><p>數量</p></td></tr><tr><td><p>香蕉|芭樂</p></td><td><p>200</p></td></tr></table>
<p>結尾段落。</p>
</body></html>
'@
[IO.File]::WriteAllText((Join-Path $work 'writer_source.html'), $writerSource, (New-Object Text.UTF8Encoding $false))
$soffice = 'C:\Program Files\LibreOffice\program\soffice.exe'
$profile = 'file:///' + ((Join-Path $work 'lo-profile').Replace('\', '/'))
& $soffice "-env:UserInstallation=$profile" --headless --infilter='HTML (StarWriter)' --convert-to odt (Join-Path $work 'writer_source.html') --outdir $work | Out-Null
& $soffice "-env:UserInstallation=$profile" --headless --convert-to 'html:HTML (StarWriter)' (Join-Path $work 'writer_source.odt') --outdir (Join-Path $work 'lo') | Out-Null
Copy-Item (Join-Path $work 'lo\writer_source.html') (Join-Path $out 'libreoffice_writer.html')
& $soffice "-env:UserInstallation=$profile" --headless --convert-to ods (Join-Path $out 'book.xlsx') --outdir $out | Out-Null

Remove-Item -Recurse -Force $work
Get-ChildItem $out | Select-Object Name, Length
