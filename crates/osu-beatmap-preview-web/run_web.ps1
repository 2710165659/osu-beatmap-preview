#Requires -Version 5.1
<#
osu! 谱面预览 Web 站点的一键构建与启动。

依次执行：安装前端依赖（仅在缺 node_modules 时）→ 构建 wasm（public/pkg）→
构建前端（dist/）→ 前台启动 backend/server.js。脚本内部所有路径都基于脚本自身
所在目录解析，但不会切换调用者的工作目录。

用法：
  .\run_web.ps1                        构建全部并启动（默认 http://127.0.0.1:8787）
  .\run_web.ps1 --port=8443 --https    构建全部，并把参数转发给后端启动
  .\run_web.ps1 -NoWasm                跳过 wasm 构建（只改了前端时用，快很多）
  .\run_web.ps1 -NoBuild               跳过前端构建（只改了 wasm，或只想起服务）
  .\run_web.ps1 -NoInstall             不安装前端依赖（缺 node_modules 时直接报错）
  .\run_web.ps1 -NoServe               只构建，不启动服务
  .\run_web.ps1 -Help                  显示这份说明

后端自身的参数（--host / --port / --cache-dir / --https / --tls-cert / --tls-key /
--tls-host / --no-cache / --quiet / --help）会原样转发。

本机禁用了脚本执行策略时改成：
  powershell -ExecutionPolicy Bypass -File .\run_web.ps1
#>

# 不用 `$ErrorActionPreference = 'Stop'`：Windows PowerShell 5.1 下原生命令写 stderr
# （cargo 的编译进度就是 stderr）会被当成终止错误，构建会莫名其妙中断。这里统一在
# `Invoke-Native` 里看退出码。
$ErrorActionPreference = 'Continue'

# 方案 A：记录项目根目录，但不切换调用者的工作目录。
$projectRoot = $PSScriptRoot

# node / npm / cargo 的输出是 UTF-8（后端日志也是中文），控制台默认代码页不是 65001 时
# PowerShell 会按默认代码页解码，中文全是乱码。这里显式指定按 UTF-8 解码。
$previousOutputEncoding = $null
try {
    $previousOutputEncoding = [Console]::OutputEncoding
    [Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
} catch {
    # 某些宿主（例如没有控制台的自动化环境）不允许改这个属性，忽略即可。
}

function Write-Usage {
    @'
osu! 谱面预览 Web 站点的一键构建与启动。

  run_web.ps1 [选项] [后端参数...]

选项：
  -NoWasm      跳过 wasm 构建（只改了前端时用）
  -NoBuild     跳过前端构建（只改了 wasm 时用）
  -NoInstall   不安装前端依赖（缺 node_modules 时直接报错）
  -NoServe     只构建，不启动服务
  -Help        显示这份说明

后端参数（--port / --host / --https / --cache-dir ...）原样转发给 backend/server.js，
例如：.\run_web.ps1 --port=8443 --https
'@ | Write-Host
}

# 执行一条外部命令；退出码非 0 时抛出，由下面的 catch 统一收尾。
function Invoke-Native {
    param(
        [Parameter(Mandatory = $true)][string]$Command,
        [string[]]$Arguments = @()
    )
    & $Command @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Command 退出码 $LASTEXITCODE"
    }
}

# 在项目根目录下执行一条外部命令，执行完立刻切回调用者原来的工作目录。
# npm / cargo 这类工具需要在项目目录里跑才能找到 package.json、Cargo.toml，
# 用 Push/Pop 保证对调用者的 pwd 无副作用。
function Invoke-InProject {
    param(
        [Parameter(Mandatory = $true)][string]$Command,
        [string[]]$Arguments = @()
    )
    Push-Location -LiteralPath $projectRoot
    try {
        Invoke-Native -Command $Command -Arguments $Arguments
    } finally {
        Pop-Location
    }
}

function Test-Tool {
    param([Parameter(Mandatory = $true)][string]$Name)
    return [bool](Get-Command -Name $Name -ErrorAction SilentlyContinue)
}

# 解析要调用的 npm：必须优先用 `npm.cmd`。
#
# PowerShell 会把 `npm` 解析到 Node.js 自带的 `npm.ps1` 垫片，而该垫片在「从一个脚本行里
# 被调用」时会重新解析调用行文本、按调用名字长度截断来取参数（见 npm.ps1 里的
# `$MyInvocation.Line` 分支）。于是 `& $Command @Arguments` 这样的调用会被截成
# `<参数名> ...`，npm 拿到的是 `Command` 之类的垃圾参数（实测 npm 10.9.3 + PowerShell 7
# 报 `Unknown command: "Command"`）。`npm.cmd` 没有这个分支，参数原样传递。
function Resolve-NpmCommand {
    foreach ($candidate in @('npm.cmd', 'npm')) {
        if (Test-Tool $candidate) {
            return $candidate
        }
    }
    throw '找不到 npm。它随 Node.js 一起安装，请检查 PATH。'
}

# 解析参数：本脚本的开关自己处理，其余原样交给后端（`--port=8443` 也走这条路）。
$noInstall = $false
$noWasm = $false
$noBuild = $false
$noServe = $false
$serverArgs = New-Object System.Collections.Generic.List[string]

foreach ($argument in $args) {
    if ($argument -eq '-NoInstall') {
        $noInstall = $true
    } elseif ($argument -eq '-NoWasm') {
        $noWasm = $true
    } elseif ($argument -eq '-NoBuild') {
        $noBuild = $true
    } elseif ($argument -eq '-NoServe') {
        $noServe = $true
    } elseif ($argument -in @('-Help', '-?', '/?')) {
        Write-Usage
        return
    } else {
        [void]$serverArgs.Add($argument)
    }
}

try {
    # -----------------------------------------------------------------------
    # 0. 运行环境检查：缺什么就明确说清楚，不要让调用方去猜 npm/cargo 的报错。
    # -----------------------------------------------------------------------
    if (-not (Test-Tool 'node')) {
        throw '找不到 node。请先安装 Node.js 20.19+：https://nodejs.org/'
    }
    $nodeVersion = [string](& node -p 'process.versions.node' | Select-Object -First 1)
    $nodeVersion = $nodeVersion.Trim()
    $nodeMajor = 0
    if ($nodeVersion -match '^(\d+)') {
        $nodeMajor = [int]$Matches[1]
    }
    if ($nodeMajor -lt 20) {
        throw "Node.js 版本过低（$nodeVersion），Vite 7 需要 20.19+。"
    }
    if (-not (Test-Tool 'npm')) {
        throw '找不到 npm。它随 Node.js 一起安装，请检查 PATH。'
    }
    $npmCommand = Resolve-NpmCommand

    if (-not $noWasm) {
        if (-not (Test-Tool 'cargo')) {
            throw '找不到 cargo。构建 wasm 需要 Rust 工具链：https://rustup.rs/'
        }
        if (-not (Test-Tool 'wasm-bindgen')) {
            throw @'
找不到 wasm-bindgen，它的版本必须与 Cargo.lock 里的 wasm-bindgen 一致：
  cargo install wasm-bindgen-cli --version <版本> --locked
版本号与完整步骤见 README.md 的「开发」一节，或用 -NoWasm 跳过。
'@
        }
        # 缺 wasm32 目标时自动补上；没有 rustup 就交给 cargo 自己报错。
        if (Test-Tool 'rustup') {
            $targets = & rustup target list --installed
            if ($LASTEXITCODE -eq 0 -and -not ($targets -match 'wasm32-unknown-unknown')) {
                Write-Host '=== 添加 Rust 目标 wasm32-unknown-unknown ...'
                Invoke-Native -Command 'rustup' -Arguments @('target', 'add', 'wasm32-unknown-unknown')
            }
        }
    }

    # -----------------------------------------------------------------------
    # 1. 前端依赖：只在缺 node_modules 时装，避免每次构建都等 npm。
    #    路径一律基于 $projectRoot，与调用者的工作目录无关。
    # -----------------------------------------------------------------------
    $nodeModulesPath = Join-Path $projectRoot 'node_modules'
    $packageLockPath = Join-Path $projectRoot 'package-lock.json'
    if (-not (Test-Path -LiteralPath $nodeModulesPath)) {
        if ($noInstall) {
            throw 'node_modules 不存在，但指定了 -NoInstall。请先去掉该参数运行一次。'
        }
        if (Test-Path -LiteralPath $packageLockPath) {
            Write-Host '=== 安装前端依赖（npm ci）...'
            Invoke-InProject -Command $npmCommand -Arguments @('ci', '--no-audit', '--no-fund')
        } else {
            Write-Host '=== 安装前端依赖（npm install）...'
            Invoke-InProject -Command $npmCommand -Arguments @('install', '--no-audit', '--no-fund')
        }
    }

    # -----------------------------------------------------------------------
    # 2. wasm 产物：public/pkg（改了 core / renderer / wasm 就必须重建）
    # -----------------------------------------------------------------------
    if ($noWasm) {
        Write-Host '=== 跳过 wasm 构建（-NoWasm），使用现有的 public\pkg。'
    } else {
        Write-Host '=== 构建 wasm（cargo --release + wasm-bindgen）...'
        Invoke-InProject -Command $npmCommand -Arguments @('run', 'build:wasm')
    }

    # -----------------------------------------------------------------------
    # 3. 前端产物：dist/（后端只托管 dist/，改了 src/ 就必须重建）
    # -----------------------------------------------------------------------
    if ($noBuild) {
        Write-Host '=== 跳过前端构建（-NoBuild），使用现有的 dist\。'
    } else {
        Write-Host '=== 构建前端（vite build）...'
        Invoke-InProject -Command $npmCommand -Arguments @('run', 'build')
    }

    # -----------------------------------------------------------------------
    # 4. 启动后端（前台运行，日志直接打在这个窗口，Ctrl+C 停止）
    #    server.js 用绝对路径调用，工作目录保持调用者原来的，无需切目录。
    # -----------------------------------------------------------------------
    if ($noServe) {
        Write-Host '=== 构建完成，已按要求不启动服务。'
        exit 0
    }
    $serverEntry = Join-Path $projectRoot 'backend\server.js'
    $distIndex = Join-Path $projectRoot 'dist\index.html'
    if (-not (Test-Path -LiteralPath $distIndex)) {
        throw '没有找到 dist\index.html：前端构建被 -NoBuild 跳过了，但 dist\ 里没有可用产物。'
    }
    Write-Host '=== 启动服务器（Ctrl+C 停止）...'
    Write-Host '    提示：浏览器里已经开着页面时，重建后请刷新页面（wasm 只在加载时导入一次）。'
    & node $serverEntry @serverArgs
    exit $LASTEXITCODE
} catch {
    Write-Host ''
    Write-Host "[错误] $($_.Exception.Message)" -ForegroundColor Red
    Write-Host '构建失败：请按上面的提示处理后重试。'
    exit 1
} finally {
    if ($null -ne $previousOutputEncoding) {
        try {
            [Console]::OutputEncoding = $previousOutputEncoding
        } catch {
            # 还原失败不影响结果。
        }
    }
}