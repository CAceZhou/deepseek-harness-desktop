# UI 缩放目验：debug 构建 + vite(pnpm dev) + 真实运行时。
# 观测点：%LOCALAPPDATA%\DSHDesktop\ui-zoom.txt（按键 → 钩子 → IPC → 命令 → 写盘 全链路）。
# 用法：先 `pnpm dev`，再 `powershell -File scripts/verify-zoom.ps1`。
param()
$ErrorActionPreference = 'Stop'
$base = Join-Path $env:LOCALAPPDATA 'DSHDesktop'
$zoomFile = Join-Path $base 'ui-zoom.txt'
$proj = 'H:\My_Software\DSHDesktop'
$exe  = Join-Path $proj 'src-tauri\target\debug\dshdesktop.exe'
$emptyRuntime = Join-Path $env:TEMP 'dsh-empty-runtime'

function Step($m) { Write-Host "`n=== $m ===" }

function Read-Zoom {
  if (Test-Path $zoomFile) {
    try { return [double](Get-Content $zoomFile -Raw).Trim() } catch { return 1.0 }
  }
  return 1.0
}

function Expect-Zoom($expect, $label) {
  $deadline = (Get-Date).AddSeconds(6)
  $v = Read-Zoom
  while ((Get-Date) -lt $deadline) {
    $v = Read-Zoom
    if ([math]::Abs($v - $expect) -lt 0.0001) { Write-Host "OK   $label -> $v"; return }
    Start-Sleep -Milliseconds 300
  }
  throw "FAIL ${label}：期望 $expect，实际 $v"
}

Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public class ZoomWin {
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumWindowsProc cb, IntPtr lp);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] static extern void mouse_event(uint flags, int dx, int dy, uint data, IntPtr extra);
    struct RECT { public int Left, Top, Right, Bottom; }
    delegate bool EnumWindowsProc(IntPtr h, IntPtr lp);
    public static IntPtr FindMain(uint targetPid) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, lp) => {
            uint pid; GetWindowThreadProcessId(h, out pid);
            if (pid != targetPid) return true;
            var cls = new StringBuilder(256); GetClassName(h, cls, cls.Capacity);
            if (!cls.ToString().StartsWith("Tauri Window")) return true;
            if (IsWindowVisible(h) && found == IntPtr.Zero) found = h;
            return true;
        }, IntPtr.Zero);
        return found;
    }
    public static void FocusAndClick(IntPtr h) {
        ShowWindow(h, 9); // SW_RESTORE
        SetForegroundWindow(h);
        var r = new RECT(); GetWindowRect(h, out r);
        SetCursorPos((r.Left + r.Right) / 2, (r.Top + r.Bottom) / 2);
        mouse_event(0x0002, 0, 0, 0, IntPtr.Zero);
        mouse_event(0x0004, 0, 0, 0, IntPtr.Zero);
    }
}
'@

function Send-ZoomKeys($keys) {
  $p = Get-Process dshdesktop -ErrorAction SilentlyContinue | Select-Object -First 1
  if (-not $p) { throw 'dshdesktop 未运行' }
  $hwnd = [ZoomWin]::FindMain([uint32]$p.Id)
  if ($hwnd -eq [IntPtr]::Zero) { throw '找不到可见主窗口' }
  [ZoomWin]::FocusAndClick($hwnd)
  Start-Sleep -Milliseconds 500
  $wshell = New-Object -ComObject WScript.Shell
  $wshell.SendKeys($keys)
}

function Stop-All {
  Get-Process dshdesktop -ErrorAction SilentlyContinue | Stop-Process -Force
  Get-CimInstance Win32_Process -Filter "Name='node.exe'" |
    Where-Object { $_.CommandLine -like '*DSHDesktop*bin.js*' } |
    ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
}

function Wait-DshReady($timeoutSec) {
  $deadline = (Get-Date).AddSeconds($timeoutSec)
  while ((Get-Date) -lt $deadline) {
    $node = Get-CimInstance Win32_Process -Filter "Name='node.exe'" |
      Where-Object { $_.CommandLine -like '*DSHDesktop*bin.js*' } | Select-Object -First 1
    if ($node) {
      $m = [regex]::Match($node.CommandLine, '--port\s+(\d+)')
      if ($m.Success) {
        try {
          $r = Invoke-WebRequest -Uri "http://127.0.0.1:$($m.Groups[1].Value)/" -UseBasicParsing -TimeoutSec 1
          if ($r.StatusCode -eq 200) { return $m.Groups[1].Value }
        } catch {}
      }
    }
    Start-Sleep -Seconds 1
  }
  throw "dsh ${timeoutSec}s 内未就绪"
}

Step '0. 前置检查：vite dev server（debug 构建的 splash 从 localhost:5173 加载）'
try {
  $r = Invoke-WebRequest -Uri 'http://localhost:5173/' -UseBasicParsing -TimeoutSec 2
  if ($r.StatusCode -ne 200) { throw "5173 返回 $($r.StatusCode)" }
} catch { throw 'vite 未运行：请先 pnpm dev，再跑本脚本' }
Stop-All
Remove-Item $zoomFile -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $emptyRuntime | Out-Null
Start-Sleep -Seconds 1

try {
  Step '1. splash 阶段（空运行时阻断 dsh，停留 splash）'
  $env:DSHDESKTOP_RUNTIME_DIR = $emptyRuntime
  Start-Process -FilePath $exe | Out-Null
  Start-Sleep -Seconds 5  # 等 splash 加载 + 钩子注入

  Send-ZoomKeys '^+(=)'
  Expect-Zoom 1.02 'splash Ctrl+Shift+= 放大'

  Send-ZoomKeys '^(=)'
  Start-Sleep -Seconds 1
  $v = Read-Zoom
  if ([math]::Abs($v - 1.02) -gt 0.0001) { throw "FAIL Ctrl+=（无 Shift）不应触发，实际 $v" }
  Write-Host "OK   Ctrl+=（无 Shift）未触发（仍为 $v）"

  Send-ZoomKeys '^+(-)'
  Expect-Zoom 1.00 'splash Ctrl+Shift+- 缩小'

  Step '2. dsh UI 阶段（真实运行时，走远程 IPC capability）'
  Stop-All
  Start-Sleep -Seconds 2
  $env:DSHDESKTOP_RUNTIME_DIR = Join-Path $proj 'src-tauri\runtime\windows-x64'
  Start-Process -FilePath $exe | Out-Null
  $port = Wait-DshReady 60
  Write-Host "dsh 就绪 port=$port"
  Start-Sleep -Seconds 3  # 等导航 + 页面加载完成后注入钩子

  Send-ZoomKeys '^+(=)'
  Expect-Zoom 1.02 'dsh UI Ctrl+Shift+= 放大'
  Send-ZoomKeys '^+(=)'
  Expect-Zoom 1.04 'dsh UI 再放大'
  Send-ZoomKeys '^+(-)'
  Expect-Zoom 1.02 'dsh UI Ctrl+Shift+- 缩小'

  Step '3. 重启验证持久化（当前 1.02；若丢持久化则会从 1.00 起缩到 0.98）'
  Stop-All
  Start-Sleep -Seconds 2
  Start-Process -FilePath $exe | Out-Null
  $port = Wait-DshReady 60
  Write-Host "重启后 dsh 就绪 port=$port"
  Start-Sleep -Seconds 3
  Send-ZoomKeys '^+(-)'
  Expect-Zoom 1.00 '重启后缩小（证明 1.02 已从磁盘加载）'

  Step '4. 截图留证'
  & powershell -NoProfile -File (Join-Path $proj 'scripts\shot-window.ps1') | Out-Null
  $png = Join-Path $env:TEMP 'dshdesktop-full.png'
  if (Test-Path $png) { Copy-Item $png (Join-Path $env:TEMP 'dshdesktop-zoom.png') -Force }

  Write-Host "`n全部通过"
} finally {
  Step '5. 清理：停实例、还原缩放文件'
  Stop-All
  Remove-Item $zoomFile -Force -ErrorAction SilentlyContinue
}
