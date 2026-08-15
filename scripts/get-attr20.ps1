$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public class DwmGet {
    [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr hwnd, uint attr, ref int val, uint size);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumWindowsProc cb, IntPtr lp);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    delegate bool EnumWindowsProc(IntPtr h, IntPtr lp);
    public static IntPtr FindMainWindow(uint targetPid) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, lp) => {
            uint pid; GetWindowThreadProcessId(h, out pid);
            if (pid != targetPid) return true;
            var cls = new StringBuilder(256); GetClassName(h, cls, cls.Capacity);
            if (cls.ToString() == "Tauri Window" && found == IntPtr.Zero) found = h;
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
'@
$app = @(Get-Process dshdesktop)[0]
$hwnd = [DwmGet]::FindMainWindow([uint32]$app.Id)
$cur = -1
$hr = [DwmGet]::DwmGetWindowAttribute($hwnd, 20, [ref]$cur, 4)
Write-Output "hwnd=$hwnd attr20=$cur (hr=0x$($hr.ToString('X8')))"
