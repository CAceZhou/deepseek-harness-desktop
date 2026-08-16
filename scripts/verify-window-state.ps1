# E2E check: window size/position must survive an app restart.
# Flow: launch debug exe -> resize main window via SetWindowPos -> quit via WM_CLOSE
# (close_behavior=quit) -> relaunch -> assert the rect matches the resized one.
# Prereq: scripts/use-fixture-runtime.ps1 has been run and `cargo build` succeeded.
# Backs up the real settings.json / window-state.json and restores them afterwards.
$ErrorActionPreference = 'Stop'

$root = Resolve-Path (Join-Path $PSScriptRoot '..')
$exe = Join-Path $root 'src-tauri\target\debug\dshdesktop.exe'
$runtimeDir = Join-Path $root 'src-tauri\runtime\windows-x64'
if (-not (Test-Path $exe)) { throw "debug exe not found: $exe (run cargo build first)" }
if (-not (Test-Path (Join-Path $runtimeDir 'node.exe'))) { throw 'fixture runtime missing (run use-fixture-runtime.ps1 first)' }

$baseDir = Join-Path $env:LOCALAPPDATA 'DSHDesktop'
$settingsFile = Join-Path $baseDir 'settings.json'
$settingsBak = "$settingsFile.wstest-bak"
$stateDir = Join-Path $env:APPDATA 'com.dshdesktop.desktop'
$stateFile = Join-Path $stateDir '.window-state.json'
$stateBak = "$stateFile.wstest-bak"

Add-Type @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public class Win32 {
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hWnd, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hWnd, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern int GetClassName(IntPtr hWnd, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);
    public delegate bool EnumProc(IntPtr h, IntPtr l);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    // MainWindowHandle 不可靠：debug exe 还持有可见控制台与 Tao/托盘辅助窗口。
    // 主窗口是唯一 class 为 "Tauri Window" 的顶层窗口。
    public static IntPtr FindMain(uint wantPid) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, l) => {
            uint pid; GetWindowThreadProcessId(h, out pid);
            if (pid != wantPid) return true;
            var c = new StringBuilder(256); GetClassName(h, c, 256);
            if (c.ToString() == "Tauri Window") { found = h; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
"@

function Get-MainRect {
    $p = Get-Process dshdesktop -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $p) { return $null }
    $h = [Win32]::FindMain($p.Id)
    if ($h -eq [IntPtr]::Zero -or -not [Win32]::IsWindowVisible($h)) { return $null }
    $r = New-Object Win32+RECT
    [void][Win32]::GetWindowRect($h, [ref]$r)
    return @{ Proc = $p; Hwnd = $h; X = $r.Left; Y = $r.Top; W = ($r.Right - $r.Left); H = ($r.Bottom - $r.Top) }
}

function Wait-MainWindow([int]$seconds) {
    $deadline = (Get-Date).AddSeconds($seconds)
    while ((Get-Date) -lt $deadline) {
        $rect = Get-MainRect
        if ($rect) { return $rect }
        Start-Sleep -Milliseconds 500
    }
    throw 'main window did not appear in time'
}

$proc = $null
try {
    New-Item -ItemType Directory -Force $baseDir | Out-Null
    if (Test-Path $settingsFile) { Copy-Item $settingsFile $settingsBak -Force }
    '{"close_behavior":"quit"}' | Out-File -Encoding ascii $settingsFile
    if (Test-Path $stateFile) { Copy-Item $stateFile $stateBak -Force; Remove-Item $stateFile -Force }

    $env:DSHDESKTOP_RUNTIME_DIR = $runtimeDir

    # --- first launch: resize, then quit ---
    $proc = Start-Process $exe -PassThru
    $null = Wait-MainWindow 30
    Start-Sleep -Seconds 2
    $targetW = 1000; $targetH = 700; $targetX = 200; $targetY = 150
    $r = Get-MainRect
    [void][Win32]::SetWindowPos($r.Hwnd, [IntPtr]::Zero, $targetX, $targetY, $targetW, $targetH, 0)
    Start-Sleep -Seconds 1
    $afterResize = Get-MainRect
    Write-Host ("after resize: {0}x{1} @ ({2},{3})" -f $afterResize.W, $afterResize.H, $afterResize.X, $afterResize.Y)

    [void][Win32]::PostMessage($afterResize.Hwnd, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)  # WM_CLOSE 直发主窗口句柄（CloseMainWindow 走 MainWindowHandle，可能误中控制台窗口）
    $deadline = (Get-Date).AddSeconds(20)
    while (-not $afterResize.Proc.HasExited -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
    if (-not $afterResize.Proc.HasExited) { throw 'app did not exit after WM_CLOSE' }
    $proc = $null

    if (-not (Test-Path $stateFile)) { throw "window-state.json was not written: $stateFile" }
    Write-Host '--- window-state.json ---'
    Get-Content $stateFile | Write-Host

    # --- second launch: geometry must be restored ---
    $proc = Start-Process $exe -PassThru
    $restored = Wait-MainWindow 30
    Start-Sleep -Seconds 2
    $restored = Get-MainRect
    Write-Host ("restored:     {0}x{1} @ ({2},{3})" -f $restored.W, $restored.H, $restored.X, $restored.Y)

    $dw = [Math]::Abs($restored.W - $afterResize.W); $dh = [Math]::Abs($restored.H - $afterResize.H)
    $dx = [Math]::Abs($restored.X - $afterResize.X); $dy = [Math]::Abs($restored.Y - $afterResize.Y)
    if ($dw -le 2 -and $dh -le 2 -and $dx -le 2 -and $dy -le 2) {
        Write-Host 'PASS: window geometry restored after restart'
    } else {
        throw "FAIL: geometry not restored (dW=$dw dH=$dh dX=$dx dY=$dy)"
    }
} finally {
    if ($proc -and -not $proc.HasExited) { $proc.Kill() }
    Get-Process dshdesktop -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    if (Test-Path $settingsBak) { Move-Item $settingsBak $settingsFile -Force }
    elseif (Test-Path $settingsFile) { Remove-Item $settingsFile -Force }
    if (Test-Path $stateBak) { Move-Item $stateBak $stateFile -Force }
}
