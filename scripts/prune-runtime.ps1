# 精简运行时体积：删除 node_modules 中运行时不需要的文件。
# 用法：powershell -File scripts/prune-runtime.ps1 [-RuntimeDir ..\src-tauri\runtime\windows-x64] [-WhatIf]
[CmdletBinding()]
param(
  [string]$RuntimeDir,
  [switch]$WhatIf
)
$ErrorActionPreference = 'Stop'

if (-not $RuntimeDir) {
  $RuntimeDir = Join-Path $PSScriptRoot "..\src-tauri\runtime\windows-x64"
}
$nm = Join-Path (Resolve-Path $RuntimeDir) 'dsh\node_modules'
if (-not (Test-Path $nm)) { throw "node_modules 不存在：$nm" }

$removedBytes = 0L
$removedCount = 0
function Remove-Item-Counted($path) {
  $script:removedCount++
  if (Test-Path $path) {
    $size = (Get-ChildItem $path -Recurse -File -Force -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum
    $script:removedBytes += $size
    if (-not $WhatIf) { Remove-Item $path -Recurse -Force -ErrorAction SilentlyContinue }
  }
}

# 1. 通用规则：文档/测试/示例/类型声明/sourcemap
$dirNames = @('test', 'tests', '__tests__', 'docs', 'doc', 'example', 'examples', 'coverage', '.github', '.vscode')
$filePatterns = @('*.d.ts', '*.d.mts', '*.d.cts', '*.map', '*.md', '*.markdown',
  'LICENSE', 'LICENCE', 'LICENSE.*', 'LICENCE.*', 'COPYING*', 'NOTICE*',
  'CHANGELOG*', 'HISTORY*', 'CONTRIBUTING*', 'CODE_OF_CONDUCT*', 'SECURITY*', 'AUTHORS*',
  '.npmignore', '.eslint*', '.prettier*', 'tsconfig.json', 'karma.conf.js', '.travis.yml')

Get-ChildItem $nm -Directory | ForEach-Object { $pkg = $_.FullName
  # 作用域包（@x/y）需深入一层
  $targets = if ($_.Name.StartsWith('@')) { Get-ChildItem $pkg -Directory | ForEach-Object FullName } else { @($pkg) }
  foreach ($t in $targets) {
    foreach ($d in $dirNames) { Remove-Item-Counted (Join-Path $t $d) }
    Get-ChildItem $t -Recurse -File -Force -Include $filePatterns -ErrorAction SilentlyContinue |
      Where-Object { $_.FullName -notmatch '\\(test|tests|__tests__|docs?|examples?)\\' } | # 目录规则已覆盖，避免重复计数
      ForEach-Object {
        $script:removedCount++
        $script:removedBytes += $_.Length
        if (-not $WhatIf) { Remove-Item $_.FullName -Force -ErrorAction SilentlyContinue }
      }
  }
}

# 2. node-pty 预编译二进制：只保留 win32-x64（dsh 终端功能依赖它）
$prebuilds = Join-Path $nm 'node-pty\prebuilds'
foreach ($sub in @('darwin-arm64', 'darwin-x64', 'win32-arm64')) {
  Remove-Item-Counted (Join-Path $prebuilds $sub)
}
# node-pty 的 C++ 源码/构建中间产物（运行时只用 prebuilds/build 里的 .node）
foreach ($sub in @('src', 'deps', 'third_party', 'typings', 'scripts')) {
  Remove-Item-Counted (Join-Path $nm "node-pty\$sub")
}

# 3. sharp 的 wasm 回退包：win32-x64 原生包存在时用不到
if (Test-Path (Join-Path $nm '@img\sharp-win32-x64')) {
  Remove-Item-Counted (Join-Path $nm '@img\sharp-wasm32')
}

$mb = [math]::Round($removedBytes / 1MB, 1)
Write-Output ("{2}删除 {0} 项，共 {1} MB" -f $removedCount, $mb, $(if ($WhatIf) { '[预演] 将' } else { '已' }))
