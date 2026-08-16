# E2E check: dsh 回合完成（turn/end + completed）在主窗口隐藏时应触发 Windows 通知，
# 子代理会话不触发。链路：fixture(fake-dsh.cjs) 每 2s 推一轮回合事件 → 壳 WS 订阅
# → 分类/过滤 → sink → toast。toast 本身难以脚本化断言，改为断言 sink 写入 events.log
# 的 "Notify: TurnCompleted ..." 行（该行只在窗口隐藏且设置开启时写）。
# Prereq: scripts/use-fixture-runtime.ps1 已跑过且 cargo build 成功。
# 会备份并恢复真实 settings.json；结束后清杀进程树。
$ErrorActionPreference = 'Stop'

$root = Resolve-Path (Join-Path $PSScriptRoot '..')
$exe = Join-Path $root 'src-tauri\target\debug\dshdesktop.exe'
$runtimeDir = Join-Path $root 'src-tauri\runtime\windows-x64'
if (-not (Test-Path $exe)) { throw "debug exe not found: $exe (run cargo build first)" }
if (-not (Test-Path (Join-Path $runtimeDir 'node.exe'))) { throw 'fixture runtime missing (run use-fixture-runtime.ps1 first)' }

$baseDir = Join-Path $env:LOCALAPPDATA 'DSHDesktop'
$settingsFile = Join-Path $baseDir 'settings.json'
$settingsBak = "$settingsFile.cntest-bak"
$logFile = Join-Path $baseDir 'events.log'

Add-Type @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public class WinHideN {
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumWindowsProc cb, IntPtr lp);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    public const uint WM_CLOSE = 0x0010;
    delegate bool EnumWindowsProc(IntPtr h, IntPtr lp);
    public static IntPtr FindMainWindow(uint targetPid) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, lp) => {
            uint pid; GetWindowThreadProcessId(h, out pid);
            if (pid != targetPid || !IsWindowVisible(h)) return true;
            var cls = new StringBuilder(256); GetClassName(h, cls, cls.Capacity);
            if (cls.ToString() == "Tauri Window") found = h;
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
"@

function Wait-For([scriptblock]$pred, [int]$seconds, [string]$what) {
    $deadline = (Get-Date).AddSeconds($seconds)
    while ((Get-Date) -lt $deadline) {
        if (& $pred) { return }
        Start-Sleep -Milliseconds 500
    }
    throw "timeout waiting for $what"
}

function Read-Log { Get-Content $logFile -Raw -Encoding UTF8 -ErrorAction SilentlyContinue }

$proc = $null
try {
    New-Item -ItemType Directory -Force $baseDir | Out-Null
    if (Test-Path $settingsFile) { Copy-Item $settingsFile $settingsBak -Force }
    # 已知设置：关窗隐藏到托盘 + 完成通知开 + 默认提示音（缺省字段由 serde 补齐）
    '{"close_behavior":"background","notify_on_completion":true,"completion_sound":"default"}' |
        Out-File -Encoding ascii $settingsFile

    Get-Process dshdesktop -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Seconds 1
    $baseline = Read-Log
    $baselineLen = if ($baseline) { $baseline.Length } else { 0 }
    $newLines = { $log = Read-Log; if ($log -and $log.Length -gt $baselineLen) { $log.Substring($baselineLen) } else { '' } }

    $env:DSHDESKTOP_RUNTIME_DIR = $runtimeDir
    $proc = Start-Process $exe -PassThru

    # 主窗口出现
    Wait-For { Get-Process dshdesktop -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 } } 30 'main window'
    # dsh(fixture) 就绪：本轮新增的 events.log 里出现 Ready 状态行（只看增量，防旧日志误判）
    try {
        Wait-For { (& $newLines) -match 'Ready' } 60 'dsh Ready'
    } catch {
        Write-Output '---- 本轮 events.log 增量 ----'
        Write-Output (& $newLines)
        throw
    }
    Write-Output 'dsh(fixture) ready'

    # 隐藏主窗口（关窗 → 托盘）
    $hwnd = [WinHideN]::FindMainWindow([uint32]$proc.Id)
    if ($hwnd -eq [IntPtr]::Zero) { throw 'main window handle not found' }
    [WinHideN]::PostMessage($hwnd, [WinHideN]::WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    Write-Output "main window hidden (hwnd=$hwnd)"

    # fixture 每 2s 一轮：fx-main completed → 应出现 Notify: TurnCompleted（含标题）
    try {
        Wait-For { (& $newLines) -match 'Notify: TurnCompleted' } 20 'Notify: TurnCompleted line'
    } catch {
        Write-Output '---- 本轮 events.log 增量 ----'
        Write-Output (& $newLines)
        throw
    }
    $delta = & $newLines

    if ($delta -notmatch [regex]::Escape('「fx 主会话」')) { throw '完成通知未带会话标题' }
    if ($delta -match '子代理') { throw '子代理会话不应触发通知' }
    Write-Output 'PASS: 隐藏态收到主会话完成通知（带标题），子代理未触发'
} finally {
    if ($proc -and -not $proc.HasExited) {
        & taskkill /T /F /PID $proc.Id 2>$null | Out-Null
    }
    if (Test-Path $settingsBak) { Move-Item $settingsBak $settingsFile -Force }
    Remove-Item Env:DSHDESKTOP_RUNTIME_DIR -ErrorAction SilentlyContinue
}
