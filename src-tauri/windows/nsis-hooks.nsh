; DSHDesktop NSIS hooks — 安装/卸载前清理残留运行时进程。
;
; 背景：<=0.1.8 把 node.exe（dsh web 服务）与 cloudflared.exe（远程隧道）
; 作为 DSHDesktop.exe 的普通子进程拉起，而 Tauri 自带的 CheckIfAppIsRunning
; 只杀主程序。主程序被强杀后这些子进程成为孤儿，仍占用
; <install>\runtime\windows-x64 下的文件，重装时报
; "Can't write: ...\cloudflared.exe" 中止。
; >=0.1.9 起子进程全部挂进 KILL_ON_JOB_CLOSE Job 随父进程退出被内核回收；
; 这里的钩子是清理旧版本遗留孤儿的兜底路径，长期保留。

!macro DSHDESKTOP_KILL_STRAY_RUNTIME_PROCESSES
  ; 1) 主程序仍在运行：连整棵子进程树一起杀（/T 树、/F 强制）
  nsExec::ExecToStack '"$SYSDIR\taskkill.exe" /F /T /IM DSHDesktop.exe'
  Pop $0
  Pop $1
  ; 2) 主程序已被强杀、只剩孤儿：按可执行路径清扫 $INSTDIR 下的所有残留进程
  ;    （runtime 里的 node.exe / cloudflared.exe 及 dsh 经内嵌 node 拉起的帮助进程）
  nsExec::ExecToStack "$SYSDIR\WindowsPowerShell\v1.0\powershell.exe -NoProfile -ExecutionPolicy Bypass -Command $\"Get-CimInstance Win32_Process | Where-Object { $$_.ExecutablePath -like '$INSTDIR\*' } | ForEach-Object { Stop-Process -Id $$_.ProcessId -Force -ErrorAction SilentlyContinue }$\""
  Pop $0
  Pop $1
  ; 等内核回收文件句柄，避免紧跟着的写/删文件仍被占用
  Sleep 1500
!macroend

!macro NSIS_HOOK_PREINSTALL
  DetailPrint "Stopping DSHDesktop background processes..."
  !insertmacro DSHDESKTOP_KILL_STRAY_RUNTIME_PROCESSES
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DetailPrint "Stopping DSHDesktop background processes..."
  !insertmacro DSHDESKTOP_KILL_STRAY_RUNTIME_PROCESSES
!macroend
