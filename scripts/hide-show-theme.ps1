# 隐藏→显示回归：对主窗口发 WM_CLOSE（应用应隐藏到托盘），等待 theme follower 在隐藏态应用，
# 再由 shot-window.ps1 强制显示并截图，验证标题栏主题在"隐藏期应用"后仍然正确。
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;

public class WinHide {
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
'@

$app = @(Get-Process dshdesktop)[0]
if (-not $app) { throw 'dshdesktop 未运行' }
$hwnd = [WinHide]::FindMainWindow([uint32]$app.Id)
if ($hwnd -eq [IntPtr]::Zero) { Write-Output '主窗口本就处于隐藏态（无需关闭）' } else {
  [WinHide]::PostMessage($hwnd, [WinHide]::WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
  Write-Output "已对主窗口 hwnd=$hwnd 发送 WM_CLOSE"
}
Start-Sleep -Seconds 2
$hwnd2 = [WinHide]::FindMainWindow([uint32]$app.Id)
Write-Output ("关闭后可见主窗口: " + $(if ($hwnd2 -eq [IntPtr]::Zero) { '无（已隐藏到托盘，OK）' } else { "仍有 $hwnd2（未隐藏？）" }))
Write-Output '等待 theme follower 在隐藏态轮询两轮…'
Start-Sleep -Seconds 5
Write-Output '现在强制显示并截图：'
& powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'shot-window.ps1')
