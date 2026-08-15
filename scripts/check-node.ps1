$nodes = Get-CimInstance Win32_Process -Filter "Name='node.exe'" | Where-Object { $_.CommandLine -like '*DSHDesktop*bin.js*' }
foreach ($n in $nodes) {
  Write-Output "node pid=$($n.ProcessId)"
  Write-Output "  cmd=$($n.CommandLine)"
}
if (-not $nodes) { Write-Output 'dsh node 未运行' }
Write-Output "--- LOCALAPPDATA runtime 目录: $(Test-Path "$env:LOCALAPPDATA\DSHDesktop\runtime")"
Get-ChildItem "$env:LOCALAPPDATA\DSHDesktop" -Directory -ErrorAction SilentlyContinue | ForEach-Object {
  $mb = [math]::Round(((Get-ChildItem $_.FullName -Recurse -File -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum / 1MB), 1)
  Write-Output "  $($_.Name)  ${mb}MB"
}
