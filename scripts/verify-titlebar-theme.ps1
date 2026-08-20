# 标题栏主题即时重绘回归：切换 dsh 主题（settings.yaml 的 ui-theme.preference）后，
# 不点击/不激活窗口，标题栏应随 theme follower 轮询（≤2s）立刻重绘。
# 旧行为（bug）：DwmSetWindowAttribute 只改属性不触发非客户区重绘，标题栏要等
# 下一次激活（点击）才变色——本脚本在旧代码上应在第 2 步超时失败
# （DWM attr20 已落但亮度不变，即"属性对了没重绘"的现场）。
# 用法：先 `pnpm dev`（debug 构建的页面从 localhost:5173 加载），
# 再 `powershell -File scripts/verify-titlebar-theme.ps1`。
param()
$ErrorActionPreference = 'Stop'
$proj = 'H:\My_Software\DSHDesktop'
$exe  = Join-Path $proj 'src-tauri\target\debug\dshdesktop.exe'
$yaml = Join-Path $env:LOCALAPPDATA 'DSHDesktop\dsh-home\settings.yaml'

function Step($m) { Write-Host "`n=== $m ===" }

Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public class TbWin {
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumWindowsProc cb, IntPtr lp);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr hwnd, uint attr, ref int val, uint size);
    public struct RECT { public int Left, Top, Right, Bottom; }
    delegate bool EnumWindowsProc(IntPtr h, IntPtr lp);
    public static IntPtr FindMainWindow(uint targetPid) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, lp) => {
            uint pid; GetWindowThreadProcessId(h, out pid);
            if (pid != targetPid || !IsWindowVisible(h)) return true;
            var cls = new StringBuilder(256); GetClassName(h, cls, cls.Capacity);
            if (cls.ToString() == "Tauri Window" && found == IntPtr.Zero) found = h;
            return true;
        }, IntPtr.Zero);
        return found;
    }
    // 提到最前但不激活（SWP_NOACTIVATE）：激活会触发非客户区重绘，掩盖 bug 现场。
    // TOPMOST→NOTOPMOST 来回一次把窗口抬到普通窗口之上，供截屏采样。
    public static void RaiseNoActivate(IntPtr h) {
        const uint flags = 0x0001 | 0x0002 | 0x0010; // NOSIZE | NOMOVE | NOACTIVATE
        SetWindowPos(h, new IntPtr(-1), 0, 0, 0, 0, flags);
        SetWindowPos(h, new IntPtr(-2), 0, 0, 0, 0, flags);
    }
    public static int DarkAttr(IntPtr h) { int v = -1; DwmGetWindowAttribute(h, 20, ref v, 4); return v; }
}
'@

# 标题栏中右段（避开左侧标题文字与右侧三按钮）截一条带，缩成 1x1 取平均亮度。
function Get-CaptionBrightness([IntPtr]$hwnd) {
  $r = New-Object TbWin+RECT
  [TbWin]::GetWindowRect($hwnd, [ref]$r) | Out-Null
  $w = $r.Right - $r.Left
  $x0 = $r.Left + [int]($w * 0.55)
  $sw = [Math]::Min(240, $r.Right - 150 - $x0); if ($sw -lt 30) { $sw = 30 }
  $y0 = $r.Top + 3
  $bmp = New-Object System.Drawing.Bitmap $sw, 22
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($x0, $y0, 0, 0, $bmp.Size)
  $one = New-Object System.Drawing.Bitmap 1, 1
  $g1 = [System.Drawing.Graphics]::FromImage($one)
  $g1.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
  $g1.DrawImage($bmp, 0, 0, 1, 1)
  $c = $one.GetPixel(0, 0)
  $g1.Dispose(); $one.Dispose(); $g.Dispose(); $bmp.Dispose()
  return (0.299 * $c.R + 0.587 * $c.G + 0.114 * $c.B)
}

function Write-Preference($pref) {
  # yaml-rust 不接受 BOM：必须无 BOM UTF-8
  $enc = New-Object System.Text.UTF8Encoding($false)
  [System.IO.File]::WriteAllText($yaml, "ui-theme:`n  preference: $pref`n", $enc)
}

function Wait-Caption($expectDark, $label, $timeoutSec) {
  $p = Get-Process dshdesktop -ErrorAction SilentlyContinue | Select-Object -First 1
  if (-not $p) { throw 'dshdesktop 未运行' }
  $deadline = (Get-Date).AddSeconds($timeoutSec)
  $hwnd = [IntPtr]::Zero
  $b = -1.0
  while ((Get-Date) -lt $deadline) {
    # 窗口可能尚未 show（启动/页面加载中）：查找放进轮询里一起等
    $hwnd = [TbWin]::FindMainWindow([uint32]$p.Id)
    if ($hwnd -ne [IntPtr]::Zero) {
      [TbWin]::RaiseNoActivate($hwnd)
      $b = Get-CaptionBrightness $hwnd
      if (($b -lt 128) -eq $expectDark) {
        Write-Host ("OK   {0} -> 亮度 {1:N0}（DWM attr20={2}）" -f $label, $b, [TbWin]::DarkAttr($hwnd))
        return
      }
    }
    Start-Sleep -Milliseconds 400
  }
  if ($hwnd -eq [IntPtr]::Zero) { throw "FAIL ${label}：${timeoutSec}s 内找不到可见主窗口" }
  $expectName = $(if ($expectDark) { '深色' } else { '浅色' })
  throw ("FAIL {0}：期望{1}标题栏，{2}s 不点击等待后亮度仍为 {3:N0}（DWM attr20={4}）" -f $label, $expectName, $timeoutSec, $b, [TbWin]::DarkAttr($hwnd))
}

function Stop-All {
  Get-Process dshdesktop -ErrorAction SilentlyContinue | Stop-Process -Force
  Get-CimInstance Win32_Process -Filter "Name='node.exe'" |
    Where-Object { $_.CommandLine -like '*DSHDesktop*bin.js*' } |
    ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
}

Step '0. 前置检查：vite dev server + debug exe'
try {
  $r = Invoke-WebRequest -Uri 'http://localhost:5173/' -UseBasicParsing -TimeoutSec 2
  if ($r.StatusCode -ne 200) { throw "5173 返回 $($r.StatusCode)" }
} catch { throw 'vite 未运行：请先 pnpm dev，再跑本脚本' }
if (-not (Test-Path $exe)) { throw "debug exe 不存在：$exe（先 cargo build）" }
Stop-All
Start-Sleep -Seconds 1

# settings.yaml 是用户真实 dsh-home 的配置：备份，结束后还原
$had = Test-Path $yaml
$backup = "$yaml.titlebar-bak"
if ($had) { Copy-Item $yaml $backup -Force }
New-Item -ItemType Directory -Force (Split-Path $yaml) | Out-Null

try {
  Step '1. 固定浅色启动（起始态确定，首轮断言即使旧代码也应过）'
  Write-Preference 'light'
  $env:DSHDESKTOP_RUNTIME_DIR = Join-Path $proj 'src-tauri\runtime\windows-x64'
  Start-Process -FilePath $exe | Out-Null
  Wait-Caption $false '启动后浅色标题栏' 30

  Step '2. 切深色：不点击窗口，标题栏应随轮询即时重绘'
  Write-Preference 'dark'
  Wait-Caption $true '深色标题栏即时切换' 12

  Step '3. 切回浅色：双向都不应需要点击'
  Write-Preference 'light'
  Wait-Caption $false '浅色标题栏即时切换' 12

  Write-Host "`n全部通过"
} finally {
  Step '4. 清理：停实例、还原 settings.yaml'
  Stop-All
  if ($had) { Copy-Item $backup $yaml -Force; Remove-Item $backup -Force }
  else { Remove-Item $yaml -Force -ErrorAction SilentlyContinue }
}
