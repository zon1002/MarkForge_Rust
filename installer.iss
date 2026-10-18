; MarkForge Markdown 工具組 v2（Rust 版）安裝程式。先執行 publish.ps1 產生 publish\。
; 與 C#／.NET 版（v1.x）使用相同的 AppId：安裝 v2 會直接升級取代 v1，並清掉 v1 的子資料夾與舊的預覽處理常式登錄。

#define AppName      "MarkForge Markdown 工具組"
#define AppNameEn    "MarkForge"
#define AppVersion   "2.0.0"
#define AppPublisher "Fu Project"
#define EditorExe    "MarkForgeEditor.exe"
#define ViewerExe    "MarkForgeViewer.exe"
#define PreviewClsid "{{57C7730F-A427-425D-92B1-CE2797C1C9CA}"
#define PreviewName  "MarkForge Markdown Preview Handler"
; v1.7.0（C# 版）預覽處理常式的 CLSID，升級時移除
#define OldPreviewClsid "{{05E77F8A-F696-4E53-87D1-B4DA1DCA49D6}"
; IPreviewHandler 的 ShellEx 機碼／64 位元 prevhost.exe 代理程序的 AppID
#define PreviewShellEx "{{8895b1c6-b41f-4c1c-a562-0d564250836f}"
#define PrevhostAppId  "{{6d2b5079-2f0b-48dd-ab7f-97cec514d30b}"
; 與 v1.x 相同的 AppId（不含外層大括號）；解除安裝資訊在 HKLM 64 位元檢視
#ifndef AppGuid
  #define AppGuid    "{4C6B7E2D-9A3F-4D8E-B6C1-7F2A5E9D0B31}"
#endif
#ifndef UninstRoot
  #define UninstRoot "HKLM64"
#endif

[Setup]
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher={#AppPublisher}
AppId={{#AppGuid}
; 舊版已在安裝前完整移除；一律建立全新的解除安裝記錄
UninstallLogMode=overwrite
DefaultDirName={autopf}\{#AppNameEn}
DefaultGroupName={#AppName}
OutputDir=installer_output
OutputBaseFilename=MarkForge_Setup_v{#AppVersion}
SetupIconFile=crates\core\assets\editor.ico
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64os
ArchitecturesInstallIn64BitMode=x64os
PrivilegesRequired=admin
UninstallDisplayIcon={app}\{#EditorExe}
; 安裝／解除安裝後通知檔案總管重新讀取檔案關聯（含預覽窗格處理常式）
ChangesAssociations=yes

[Code]
var
  DepsDownloadPage: TDownloadWizardPage;
  NeedsWebView2: Boolean;

function IsWebView2Installed(): Boolean;
begin
  Result := DirExists(ExpandConstant('{pf}\Microsoft\EdgeWebView\Application')) or
            DirExists(ExpandConstant('{pf32}\Microsoft\EdgeWebView\Application'));
end;

procedure InitializeWizard;
begin
  NeedsWebView2 := not IsWebView2Installed();
  if NeedsWebView2 then
    DepsDownloadPage := CreateDownloadPage('正在下載相依元件', '請稍候，下載完成後會自動安裝。', nil);
end;

function IsProcessRunning(const ExeName: String): Boolean;
var
  ResultCode: Integer;
begin
  Result := Exec(ExpandConstant('{cmd}'),
    '/C tasklist /NH /FI "IMAGENAME eq ' + ExeName + '" | find /I "' + ExeName + '" >nul',
    '', SW_HIDE, ewWaitUntilTerminated, ResultCode) and (ResultCode = 0);
end;

// 只刪除確定是本程式安裝的資料夾：解除安裝程式就在該資料夾內，且不是磁碟根目錄或系統資料夾。
function IsSafeToDelete(const Dir, UninstallerPath: String): Boolean;
var
  D: String;
begin
  D := Lowercase(RemoveBackslashUnlessRoot(Dir));
  Result := (Length(D) > 3) and
    (CompareText(RemoveBackslashUnlessRoot(ExtractFileDir(UninstallerPath)), RemoveBackslashUnlessRoot(Dir)) = 0) and
    (D <> Lowercase(ExpandConstant('{commonpf64}'))) and (D <> Lowercase(ExpandConstant('{commonpf32}'))) and
    (D <> Lowercase(ExpandConstant('{win}'))) and (D <> Lowercase(ExpandConstant('{sys}'))) and
    (D <> Lowercase(GetEnv('USERPROFILE')));
end;

// 取代舊版（含 C#／.NET 版 v1.x）：先以舊版自己的解除安裝程式靜默移除（檔案、捷徑、登錄、
// 舊的解除安裝記錄），再刪除舊安裝資料夾內剩下的所有檔案；本次安裝會建立全新的解除安裝程式，
// 日後解除安裝可完整移除 Rust 版。
procedure RemovePreviousVersion;
var
  Key, UninstallString, InstallDir, Uninstaller: String;
  ResultCode, I: Integer;
begin
  Key := 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{#AppGuid}_is1';
  if not RegQueryStringValue({#UninstRoot}, Key, 'UninstallString', UninstallString) then
    exit;
  Uninstaller := RemoveQuotes(UninstallString);
  if not RegQueryStringValue({#UninstRoot}, Key, 'InstallLocation', InstallDir) then
    InstallDir := ExtractFilePath(Uninstaller);

  if FileExists(Uninstaller) then
  begin
    Exec(Uninstaller, '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
    // 解除安裝程式會複製自己到暫存資料夾再執行第二階段；等它把自己刪掉（最多 30 秒）
    for I := 1 to 60 do
    begin
      if not FileExists(Uninstaller) then
        break;
      Sleep(500);
    end;
  end;

  if DirExists(InstallDir) and IsSafeToDelete(InstallDir, Uninstaller) then
    DelTree(RemoveBackslashUnlessRoot(InstallDir), True, True, True);
  // 舊版解除安裝失敗時登錄可能還在；刪除，避免新安裝附加到舊的解除安裝記錄
  RegDeleteKeyIncludingSubkeys({#UninstRoot}, Key);
end;

// 安裝前：要求關閉執行中的 Editor／Viewer（避免遺失未存檔的內容），結束預覽程序
// （prevhost.exe 可能載著舊版處理常式 DLL；檔案總管下次預覽時會自動重新啟動），然後移除舊版。
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  ResultCode: Integer;
begin
  if IsProcessRunning('MarkForgeEditor.exe') or IsProcessRunning('MarkForge.Editor.exe') or
     IsProcessRunning('MarkForgeViewer.exe') or IsProcessRunning('MarkForge.Viewer.exe') then
  begin
    Result := 'MarkForge Editor 或 Viewer 正在執行。請先儲存並關閉它們，再按「上一步」後重新「安裝」。';
    exit;
  end;
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM MarkForgePreviewHost.exe', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM MarkForge.PreviewHost.exe', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM prevhost.exe', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  RemovePreviousVersion;
  Result := '';
end;

// Rust 版不需要 .NET；只需要 WebView2 Runtime（Windows 11 已內建）。缺少時詢問是否自動下載安裝。
function NextButtonClick(CurPageID: Integer): Boolean;
var
  ResultCode: Integer;
begin
  Result := True;
  if (CurPageID <> wpReady) or (not NeedsWebView2) then
    exit;
  if MsgBox('偵測不到 Microsoft Edge WebView2 Runtime（預覽功能需要）。' + #13#10#13#10 +
            '是否要現在自動下載並安裝（需連線至 microsoft.com）？' + #13#10 +
            '選「否」會略過，安裝完成後請自行下載安裝。', mbConfirmation, MB_YESNO) = IDNO then
    exit;
  DepsDownloadPage.Clear;
  DepsDownloadPage.Add('https://go.microsoft.com/fwlink/p/?LinkId=2124703', 'MicrosoftEdgeWebview2Setup.exe', '');
  DepsDownloadPage.Show;
  try
    DepsDownloadPage.Download;
  except
    DepsDownloadPage.Hide;
    if not DepsDownloadPage.AbortedByUser then
      SuppressibleMsgBox('下載 WebView2 Runtime 失敗：' + GetExceptionMessage + #13#10 +
        '請安裝完成後自行至 https://developer.microsoft.com/microsoft-edge/webview2/ 下載安裝。', mbError, MB_OK, IDOK);
    exit;
  end;
  DepsDownloadPage.Hide;
  if not Exec(ExpandConstant('{tmp}\MicrosoftEdgeWebview2Setup.exe'), '/silent /install', '', SW_SHOWNORMAL, ewWaitUntilTerminated, ResultCode) then
    SuppressibleMsgBox('無法啟動 WebView2 Runtime 安裝程式。', mbError, MB_OK, IDOK)
  else if ResultCode <> 0 then
    SuppressibleMsgBox('WebView2 Runtime 安裝可能未成功完成（結束碼 ' + IntToStr(ResultCode) + '）。', mbError, MB_OK, IDOK);
end;

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "建立桌面捷徑"; GroupDescription: "其他選項:"
Name: "previewpane"; Description: "在檔案總管預覽窗格顯示 Markdown（.md／.markdown）"; GroupDescription: "檔案總管整合:"

[InstallDelete]
; 從 v1.x（C#／.NET 版）升級：移除舊版的子資料夾
Type: filesandordirs; Name: "{app}\Editor"
Type: filesandordirs; Name: "{app}\Viewer"
Type: filesandordirs; Name: "{app}\PreviewHandler"

[Files]
; 四個檔案放在同一個資料夾：預覽處理常式 DLL 會在自己旁邊找 MarkForgePreviewHost.exe
Source: "publish\MarkForgeEditor.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "publish\MarkForgeViewer.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "publish\MarkForgePreviewHost.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "publish\MarkForgePreview.dll"; DestDir: "{app}"; Flags: ignoreversion restartreplace uninsrestartdelete
Source: "README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "versions.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "THIRD_PARTY_NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\MarkForge Editor";   Filename: "{app}\{#EditorExe}"
Name: "{group}\MarkForge Viewer";   Filename: "{app}\{#ViewerExe}"
Name: "{group}\解除安裝";           Filename: "{uninstallexe}"
Name: "{commondesktop}\MarkForge Editor"; Filename: "{app}\{#EditorExe}"; Tasks: desktopicon
Name: "{commondesktop}\MarkForge Viewer"; Filename: "{app}\{#ViewerExe}"; Tasks: desktopicon

[Registry]
; 檔案關聯：.md / .markdown 預設以 Viewer 開啟，右鍵另有「以 MarkForge Editor 開啟」
Root: HKCR; Subkey: ".md"; ValueType: string; ValueName: ""; ValueData: "MarkForge.MarkdownFile"; Flags: uninsdeletevalue
Root: HKCR; Subkey: ".markdown"; ValueType: string; ValueName: ""; ValueData: "MarkForge.MarkdownFile"; Flags: uninsdeletevalue
Root: HKCR; Subkey: "MarkForge.MarkdownFile"; ValueType: string; ValueName: ""; ValueData: "Markdown 文件"; Flags: uninsdeletekey
Root: HKCR; Subkey: "MarkForge.MarkdownFile\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#ViewerExe},0"
Root: HKCR; Subkey: "MarkForge.MarkdownFile\shell\open"; ValueType: string; ValueName: ""; ValueData: "以 &MarkForge Viewer 開啟"
Root: HKCR; Subkey: "MarkForge.MarkdownFile\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#ViewerExe}"" ""%1"""
Root: HKCR; Subkey: "MarkForge.MarkdownFile\shell\editwithmarkforge"; ValueType: string; ValueName: ""; ValueData: "以 Mark&Forge Editor 開啟"
Root: HKCR; Subkey: "MarkForge.MarkdownFile\shell\editwithmarkforge"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#EditorExe},0"
Root: HKCR; Subkey: "MarkForge.MarkdownFile\shell\editwithmarkforge\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#EditorExe}"" ""%1"""

; 從 v1.7.0 升級：移除 C# 版預覽處理常式的類別登錄
Root: HKCR; Subkey: "CLSID\{#OldPreviewClsid}"; Flags: deletekey noerror
Root: HKLM; Subkey: "SOFTWARE\Microsoft\Windows\CurrentVersion\PreviewHandlers"; ValueType: none; ValueName: "{#OldPreviewClsid}"; Flags: deletevalue noerror

; 檔案總管預覽窗格處理常式（IPreviewHandler）。必須註冊在 HKLM：COM 不會在 prevhost.exe
; 代理程序中啟用只註冊在 HKCU 的類別，檔案總管會直接顯示「沒有預覽可用」。
Root: HKCR; Subkey: "CLSID\{#PreviewClsid}"; ValueType: string; ValueName: ""; ValueData: "{#PreviewName}"; Tasks: previewpane; Flags: uninsdeletekey
Root: HKCR; Subkey: "CLSID\{#PreviewClsid}"; ValueType: string; ValueName: "DisplayName"; ValueData: "{#PreviewName}"; Tasks: previewpane
Root: HKCR; Subkey: "CLSID\{#PreviewClsid}"; ValueType: string; ValueName: "AppID"; ValueData: "{#PrevhostAppId}"; Tasks: previewpane
; WebView2 無法在低完整性（Low IL）程序中執行
Root: HKCR; Subkey: "CLSID\{#PreviewClsid}"; ValueType: dword; ValueName: "DisableLowILProcessIsolation"; ValueData: 1; Tasks: previewpane
Root: HKCR; Subkey: "CLSID\{#PreviewClsid}\InprocServer32"; ValueType: string; ValueName: ""; ValueData: "{app}\MarkForgePreview.dll"; Tasks: previewpane
Root: HKCR; Subkey: "CLSID\{#PreviewClsid}\InprocServer32"; ValueType: string; ValueName: "ThreadingModel"; ValueData: "Apartment"; Tasks: previewpane
Root: HKCR; Subkey: ".md\ShellEx\{#PreviewShellEx}"; ValueType: string; ValueName: ""; ValueData: "{#PreviewClsid}"; Tasks: previewpane; Flags: uninsdeletekey
Root: HKCR; Subkey: ".markdown\ShellEx\{#PreviewShellEx}"; ValueType: string; ValueName: ""; ValueData: "{#PreviewClsid}"; Tasks: previewpane; Flags: uninsdeletekey
Root: HKLM; Subkey: "SOFTWARE\Microsoft\Windows\CurrentVersion\PreviewHandlers"; ValueType: string; ValueName: "{#PreviewClsid}"; ValueData: "{#PreviewName}"; Tasks: previewpane; Flags: uninsdeletevalue

[UninstallRun]
; 解除安裝前結束預覽程序，釋放 DLL
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM MarkForgePreviewHost.exe"; Flags: runhidden; RunOnceId: "KillPreviewHost"
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM prevhost.exe"; Flags: runhidden; RunOnceId: "KillPrevhost"

[Run]
Filename: "{app}\{#EditorExe}"; Description: "立即啟動 MarkForge Editor"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
Type: filesandordirs; Name: "{app}"
