# 用 fake-dsh fixture 组装开发用运行时（真实运行时由 fetch-runtime.ps1 下载）。
# 用法：powershell -File scripts/use-fixture-runtime.ps1
# 然后：$env:DSHDESKTOP_RUNTIME_DIR = (Resolve-Path src-tauri\runtime\windows-x64); pnpm tauri dev
$ErrorActionPreference = 'Stop'

$dest = Join-Path $PSScriptRoot '..\src-tauri\runtime\windows-x64'
$pkgDir = Join-Path $dest 'dsh\node_modules\@deepseek-ai\dsh'
$binDir = Join-Path $pkgDir 'lib'

New-Item -ItemType Directory -Force $binDir | Out-Null
Copy-Item (Get-Command node).Source (Join-Path $dest 'node.exe') -Force
Copy-Item (Join-Path $PSScriptRoot '..\src-tauri\tests\fixtures\fake-dsh.cjs') (Join-Path $binDir 'bin.js') -Force
# fixture 是 CommonJS；若目录树来自 fetch-runtime，真实 package.json 带 "type":"module"
# 会把 bin.js 当 ESM 加载而崩溃（require is not defined）。铺一个 CJS 桩 package.json 兜底。
'{"name":"@deepseek-ai/dsh","version":"0.0.0-fixture","type":"commonjs","bin":{"dsh":"lib/bin.js"}}' |
    Out-File -Encoding ascii (Join-Path $pkgDir 'package.json') -Force

Write-Host "fixture runtime ready at $dest"
Write-Host 'run: $env:DSHDESKTOP_RUNTIME_DIR = (Resolve-Path src-tauri\runtime\windows-x64); pnpm tauri dev'
