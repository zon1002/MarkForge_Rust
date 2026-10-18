# 建置發行版並打包安裝程式。
#   publish\                       四個發行檔（同一個資料夾：預覽處理常式 DLL 會在自己旁邊找 MarkForgePreviewHost.exe）
#   installer_output\MarkForge_Setup_v<版本>.exe
# 需求：Rust（stable, x86_64-pc-windows-msvc）、Inno Setup 6。
# 用法：powershell -ExecutionPolicy Bypass -File publish.ps1 [-SkipInstaller]
param([switch]$SkipInstaller)
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

$cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
if (-not (Test-Path $cargo)) { $cargo = 'cargo' }

& $cargo test --workspace --release
if ($LASTEXITCODE -ne 0) { throw '測試失敗' }
& $cargo build --workspace --release
if ($LASTEXITCODE -ne 0) { throw '建置失敗' }

$publish = Join-Path $PSScriptRoot 'publish'
if (Test-Path $publish) { Remove-Item -Recurse -Force $publish }
New-Item -ItemType Directory $publish | Out-Null
foreach ($f in 'MarkForgeEditor.exe', 'MarkForgeViewer.exe', 'MarkForgePreviewHost.exe', 'MarkForgePreview.dll') {
    Copy-Item (Join-Path $PSScriptRoot "target\release\$f") $publish
}
Get-ChildItem $publish | ForEach-Object { '{0,-28} {1,12:N0} bytes' -f $_.Name, $_.Length }

if (-not $SkipInstaller) {
    $iscc = 'C:\Program Files (x86)\Inno Setup 6\ISCC.exe'
    & $iscc (Join-Path $PSScriptRoot 'installer.iss')
    if ($LASTEXITCODE -ne 0) { throw '安裝程式編譯失敗' }
}
