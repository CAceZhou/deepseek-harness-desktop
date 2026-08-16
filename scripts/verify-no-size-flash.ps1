# E2E regression: the main window must be at the REMEMBERED geometry from its
# very first visible frame (no default-size flash before window-state restore).
# Flow: pre-write .window-state.json with a distinctive size -> launch debug exe
# -> poll the "Tauri Window" class window every 25ms -> assert its size never
# changes once visible, and equals the remembered size.
# NOTE: do NOT use Process.MainWindowHandle here — the debug exe also owns a
# visible console window and Tao/tray helper windows; MainWindowHandle can point
# at any of them. The main window is the one with class "Tauri Window".
# Prereq: scripts/use-fixture-runtime.ps1 has been run and `cargo build` succeeded.
# Backs up the real .window-state.json and restores it afterwards.
$ErrorActionPreference = 'Stop'

$root = Resolve-Path (Join-Path $PSScriptRoot '..')
$exe = Join-Path $root 'src-tauri\target\debug\dshdesktop.exe'
$runtimeDir = Join-Path $root 'src-tauri\runtime\windows-x64'
if (-not (Test-Path $exe)) { throw "debug exe not found: $exe (run cargo build first)" }
if (-not (Test-Path (Join-Path $runtimeDir 'node.exe'))) { throw 'fixture runtime missing (run use-fixture-runtime.ps1 first)' }

$stateDir = Join-Path $env:APPDATA 'com.dshdesktop.desktop'
$stateFile = Join-Path $stateDir '.window-state.json'
$stateBak = "$stateFile.nsf-bak"
$targetW = 1000; $targetH = 700; $targetX = 200; $targetY = 150

Add-Type @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public class ProbeWin {
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT r);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern int GetClassName(IntPtr hWnd, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);
    public delegate bool EnumProc(IntPtr h, IntPtr l);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    public static IntPtr FindByClass(uint wantPid, string wantClass) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, l) => {
            uint pid; GetWindowThreadProcessId(h, out pid);
            if (pid != wantPid) return true;
            var c = new StringBuilder(256); GetClassName(h, c, 256);
            if (c.ToString() == wantClass) { found = h; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
"@

try {
    New-Item -ItemType Directory -Force $stateDir | Out-Null
    if (Test-Path $stateFile) { Copy-Item $stateFile $stateBak -Force }
    ('{"main":{"width":' + $targetW + ',"height":' + $targetH + ',"x":' + $targetX + ',"y":' + $targetY +
     ',"prev_x":' + $targetX + ',"prev_y":' + $targetY + ',"maximized":false,"visible":true,"decorated":true,"fullscreen":false}}') |
        Out-File -Encoding ascii $stateFile

    Get-Process dshdesktop -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Seconds 1
    $env:DSHDESKTOP_RUNTIME_DIR = $runtimeDir
    $t0 = Get-Date
    $proc = Start-Process $exe -PassThru

    $obs = @()
    $deadline = $t0.AddSeconds(6)
    while ((Get-Date) -lt $deadline) {
        $h = [ProbeWin]::FindByClass($proc.Id, 'Tauri Window')
        if ($h -ne [IntPtr]::Zero -and [ProbeWin]::IsWindowVisible($h)) {
            $r = New-Object ProbeWin+RECT
            [void][ProbeWin]::GetWindowRect($h, [ref]$r)
            $w = $r.Right - $r.Left; $hh = $r.Bottom - $r.Top
            $ms = [int]((Get-Date) - $t0).TotalMilliseconds
            if ($obs.Count -eq 0 -or $obs[-1] -ne "${w}x${hh}") {
                $obs += "${w}x${hh}"
                Write-Host ("{0,5} ms  visible {1}x{2} @ {3},{4}" -f $ms, $w, $hh, $r.Left, $r.Top)
            }
            $lastRect = @{ W = $w; H = $hh; X = $r.Left; Y = $r.Top }
        }
        Start-Sleep -Milliseconds 25
    }

    if ($obs.Count -eq 0) { throw 'FAIL: main window never became visible' }
    if ($obs.Count -gt 1) {
        throw "FAIL: size changed while visible ($($obs -join ' -> ')) -- default-size flash regressed"
    }
    # restore sanity: observed outer rect ~= remembered inner + frame insets
    $dw = [Math]::Abs($lastRect.W - $targetW); $dh = [Math]::Abs($lastRect.H - $targetH)
    if ($dw -gt 32 -or $dh -gt 64) {
        throw "FAIL: final size $($lastRect.W)x$($lastRect.H) is not the remembered ${targetW}x${targetH} (frame insets aside)"
    }
    Write-Host 'PASS: first visible frame already has the remembered geometry (no size flash)'
} finally {
    Get-Process dshdesktop -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    if (Test-Path $stateBak) { Move-Item $stateBak $stateFile -Force }
    Remove-Item Env:DSHDESKTOP_RUNTIME_DIR -ErrorAction SilentlyContinue
}
