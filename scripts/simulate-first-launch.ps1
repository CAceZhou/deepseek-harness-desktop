# 模拟首启目验：停掉已安装实例 → 暂存 dsh-home → 启动 debug 构建 → 循环截图 → 还原现场。
$ErrorActionPreference = 'Stop'
$base = Join-Path $env:LOCALAPPDATA 'DSHDesktop'
$dshHome = Join-Path $base 'dsh-home'
$bak  = Join-Path $base 'dsh-home.bak'
$proj = 'H:\My_Software\DSHDesktop'
$exe  = Join-Path $proj 'src-tauri\target\debug\dshdesktop.exe'
$outDir = Join-Path $env:TEMP 'dsh-firstlaunch'
New-Item -ItemType Directory -Force $outDir | Out-Null

function Step($m) { Write-Host "`n=== $m ===" }

Step '1. 停止运行中的实例（dshdesktop + 其 node）'
Get-Process dshdesktop -ErrorAction SilentlyContinue | Stop-Process -Force
Get-CimInstance Win32_Process -Filter "Name='node.exe'" |
  Where-Object { $_.CommandLine -like '*DSHDesktop*bin.js*' } |
  ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
Start-Sleep -Seconds 1

Step '2. 暂存 dsh-home 模拟首启'
if (Test-Path $bak) { Remove-Item $bak -Recurse -Force }
Move-Item $dshHome $bak
if (Test-Path $dshHome) { throw 'dsh-home 仍在，无法模拟首启' }

$app = $null
try {
  Step '3. 启动 debug 构建'
  $env:DSHDESKTOP_RUNTIME_DIR = Join-Path $proj 'src-tauri\runtime\windows-x64'
  $app = Start-Process -FilePath $exe -PassThru
  Write-Host "debug app pid=$($app.Id)"

  Step '4. 循环截图（每 1s，最多 25s）'
  $shot = Join-Path $proj 'scripts\shot-window.ps1'
  for ($i = 0; $i -lt 25; $i++) {
    Start-Sleep -Seconds 1
    & powershell -NoProfile -File $shot | Out-Null
    $png = Join-Path $env:TEMP 'dshdesktop-full.png'
    if (Test-Path $png) {
      Copy-Item $png (Join-Path $outDir ("shot-{0:d2}.png" -f $i)) -Force
    }
    # dsh 就绪后窗口已导航到远程 UI，splash 截图窗口结束
    $wmi = Get-CimInstance Win32_Process -Filter "Name='node.exe'" |
      Where-Object { $_.CommandLine -like '*DSHDesktop*bin.js*' }
    if ($wmi) {
      $port = [regex]::Match($wmi[0].CommandLine, '--port\s+(\d+)').Groups[1].Value
      if ($port) {
        try {
          $r = Invoke-WebRequest -Uri "http://127.0.0.1:$port/" -UseBasicParsing -TimeoutSec 1
          if ($r.StatusCode -eq 200) { Write-Host "dsh 已就绪 port=$port，再补一张后停止"; Start-Sleep 1; & powershell -NoProfile -File $shot | Out-Null; Copy-Item (Join-Path $env:TEMP 'dshdesktop-full.png') (Join-Path $outDir 'shot-ready.png') -Force; break }
        } catch {}
      }
    }
  }
} finally {
  Step '5. 清理：停 debug 实例并还原 dsh-home'
  Get-Process dshdesktop -ErrorAction SilentlyContinue | Stop-Process -Force
  Get-CimInstance Win32_Process -Filter "Name='node.exe'" |
    Where-Object { $_.CommandLine -like '*DSHDesktop*bin.js*' } |
    ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
  Start-Sleep -Seconds 1
  if (Test-Path $dshHome) { Remove-Item $dshHome -Recurse -Force }
  Move-Item $bak $dshHome
  Remove-Item Env:\DSHDESKTOP_RUNTIME_DIR -ErrorAction SilentlyContinue
}

Step '完成'
Get-ChildItem $outDir | ForEach-Object { "  $($_.Name) $([math]::Round($_.Length/1KB))KB" }
