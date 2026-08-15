$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;

public class WinList {
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumWindowsProc cb, IntPtr lp);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    public struct RECT { public int Left, Top, Right, Bottom; }
    delegate bool EnumWindowsProc(IntPtr h, IntPtr lp);

    public static IntPtr FindVisibleByPid(uint targetPid, string classPrefix) {
        IntPtr found = IntPtr.Zero;
        IntPtr hiddenBig = IntPtr.Zero; int hiddenArea = 0;
        EnumWindows((h, lp) => {
            uint pid; GetWindowThreadProcessId(h, out pid);
            if (pid != targetPid) return true;
            var cls = new StringBuilder(256); GetClassName(h, cls, cls.Capacity);
            if (!cls.ToString().StartsWith(classPrefix)) return true;
            var ttl = new StringBuilder(512); GetWindowText(h, ttl, ttl.Capacity);
            var r = new RECT(); GetWindowRect(h, out r);
            int area = (r.Right - r.Left) * (r.Bottom - r.Top);
            Console.WriteLine("  hwnd=" + h + " cls=" + cls + " visible=" + IsWindowVisible(h) + " rect=(" + r.Left + "," + r.Top + "," + (r.Right - r.Left) + "x" + (r.Bottom - r.Top) + ") title=" + ttl);
            if (IsWindowVisible(h) && (r.Right - r.Left) > 100) {
                if (found == IntPtr.Zero) found = h;
            } else if (area > hiddenArea) { hiddenArea = area; hiddenBig = h; }
            return true;
        }, IntPtr.Zero);
        if (found != IntPtr.Zero) return found;
        // 没有可见窗口：显示最大的隐藏窗口（托盘隐藏态）
        if (hiddenBig != IntPtr.Zero) {
            ShowWindow(hiddenBig, 5); // SW_SHOW
            ShowWindow(hiddenBig, 9); // SW_RESTORE
        }
        return hiddenBig;
    }
}
'@

$app = @(Get-Process dshdesktop)[0]
Write-Output "dshdesktop pid=$($app.Id)，枚举其全部顶层窗口："
$hwnd = [WinList]::FindVisibleByPid([uint32]$app.Id, 'Tauri Window')
if ($hwnd -eq [IntPtr]::Zero) { throw '找不到主窗口（可见或隐藏）' }
Write-Output "选用 hwnd=$hwnd"
[WinList]::SetForegroundWindow($hwnd) | Out-Null
Start-Sleep -Milliseconds 500

$r = New-Object WinList+RECT
[WinList]::GetWindowRect($hwnd, [ref]$r) | Out-Null
$w = $r.Right - $r.Left; $h = $r.Bottom - $r.Top
Write-Output "window rect: $($r.Left),$($r.Top) ${w}x${h}"
if ($w -lt 100) { throw '窗口仍不可用' }

Add-Type -AssemblyName System.Drawing
$bmp = New-Object System.Drawing.Bitmap $w, $h
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($r.Left, $r.Top, 0, 0, $bmp.Size)
$full = Join-Path $env:TEMP 'dshdesktop-full.png'
$bmp.Save($full)

$strip = New-Object System.Drawing.Bitmap $w, 60
$g2 = [System.Drawing.Graphics]::FromImage($strip)
$g2.CopyFromScreen($r.Left, $r.Top, 0, 0, $strip.Size)
$bar = Join-Path $env:TEMP 'dshdesktop-titlebar.png'
$strip.Save($bar)

$xmid = [int]($w / 2)
foreach ($x in @($xmid, ($w - 60), ($w - 150), 60)) {
  $px = $bmp.GetPixel($x, 15)
  Write-Output "pixel($x,15): R=$($px.R) G=$($px.G) B=$($px.B)"
}
Write-Output "full: $full"
Write-Output "bar : $bar"
$g.Dispose(); $g2.Dispose(); $bmp.Dispose(); $strip.Dispose()
