# osu-beatmap-preview 批量渲染脚本 1：一般测试集（PNG + GIF）
#
# 用法（可从任意目录运行）：
#   powershell -File "<repo>\scripts\batch_render_general.ps1"
#
# 渲染内容：
#   - std / taiko / catch / mania 的一般测试谱面，每张渲染 png 与 gif 各一次
#   - 渲染顺序：每个模式先渲染完全部 png，再渲染 gif
# 输出：
#   - 产物写入默认输出目录的 batch-general 子文件夹：
#     %TEMP%\osu-beatmap-preview\outputs\batch-general
#   - report.txt（统一报告列 + 总耗时汇总）与 results.csv（供总计脚本生成 report.md）
#   - 渲染结束后清理 outputs 根目录散文件（不动任何子文件夹），只保留对应子文件夹的文件

param(
    [ValidateRange(100, 5000)]
    [int]$GpuSampleIntervalMs = 500
)

$ErrorActionPreference = "Continue"

# ── 路径：脚本位于 scripts/ 下，二进制与工作目录都在仓库根 ──
$repoRoot = Split-Path -Parent $PSScriptRoot
$bin = Join-Path $repoRoot "target\release\osu-beatmap-preview-cli.exe"
$outputsRoot = [System.IO.Path]::GetFullPath((Join-Path $env:TEMP "osu-beatmap-preview\outputs"))
$outdir = Join-Path $outputsRoot "batch-general"
$runId = Get-Date -Format "yyyyMMdd-HHmmss-fff"
# LOG_DIR 用每次运行的唯一值：既隔离缓存变体强制重渲染，又便于读取本次的 render.log。
$logDir = Join-Path $outdir ".logs\$runId"
$renderLog = Join-Path $logDir "render.log"

if (-not (Test-Path -LiteralPath $bin -PathType Leaf)) {
    throw "未找到 Release 二进制：$bin`n请先运行 cargo build --release。"
}

# ── 渲染前清空自己的子文件夹：避免命中输出缓存导致计时失真 ──
if (Test-Path -LiteralPath $outdir) { Remove-Item -LiteralPath $outdir -Recurse -Force }
New-Item -ItemType Directory -Force -Path $outdir | Out-Null
New-Item -ItemType Directory -Force -Path $logDir | Out-Null

function Resolve-Executable {
    param([string]$Value)

    if (Test-Path -LiteralPath $Value -PathType Leaf) {
        return (Resolve-Path -LiteralPath $Value).Path
    }
    $command = Get-Command $Value -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($command) { return $command.Source }
    return $null
}

# 优先使用 Windows 的进程级 GPU 引擎计数器；不可用时回退到 nvidia-smi 的整卡利用率。
$script:gpuCounterFailed = $false
$script:nvidiaSmi = Resolve-Executable "nvidia-smi.exe"

function Get-GpuUsageSample {
    param([int]$ProcessId)

    if (-not $script:gpuCounterFailed) {
        try {
            $needle = "pid_${ProcessId}_"
            $samples = @(
                (Get-Counter '\GPU Engine(*)\Utilization Percentage' -ErrorAction Stop).CounterSamples |
                    Where-Object { $_.Path.IndexOf($needle, [System.StringComparison]::OrdinalIgnoreCase) -ge 0 } |
                    ForEach-Object { [double]$_.CookedValue }
            )
            if ($samples.Count -gt 0) {
                # 任务管理器显示最繁忙的引擎，而不是把各引擎利用率相加到 100% 以上。
                $maximum = ($samples | Measure-Object -Maximum).Maximum
                return [pscustomobject]@{ value = [math]::Max(0.0, [double]$maximum); scope = "process" }
            }
        } catch {
            $script:gpuCounterFailed = $true
        }
    }

    if ($script:nvidiaSmi) {
        try {
            $values = @(
                & $script:nvidiaSmi --query-gpu=utilization.gpu `
                    --format=csv,noheader,nounits 2>$null |
                    ForEach-Object {
                        $parsed = 0.0
                        if ([double]::TryParse($_.Trim(), [ref]$parsed)) { $parsed }
                    }
            )
            if ($values.Count -gt 0) {
                return [pscustomobject]@{
                    value = [double](($values | Measure-Object -Maximum).Maximum)
                    scope = "system"
                }
            }
        } catch {}
    }

    return $null
}

function Add-ProcessArguments {
    param(
        [System.Diagnostics.ProcessStartInfo]$StartInfo,
        [string[]]$Arguments
    )

    # PowerShell 7 使用 ArgumentList，旧版 Windows PowerShell 回退到转义后的命令行。
    if ($StartInfo.PSObject.Properties.Name -contains "ArgumentList") {
        foreach ($argument in $Arguments) { $StartInfo.ArgumentList.Add($argument) }
        return
    }

    $quoted = foreach ($argument in $Arguments) {
        if ($argument -notmatch '[\s"]') {
            $argument
        } else {
            '"' + $argument.Replace('"', '\"') + '"'
        }
    }
    $StartInfo.Arguments = $quoted -join " "
}

# 统一进程运行器：采集墙钟、峰值内存、CPU 时间与 GPU 利用率。
function Invoke-Binary {
    param([string[]]$ArgList)

    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $bin
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.StandardOutputEncoding = [System.Text.Encoding]::UTF8
    $psi.StandardErrorEncoding = [System.Text.Encoding]::UTF8
    $psi.UseShellExecute = $false
    $psi.WorkingDirectory = $repoRoot
    Add-ProcessArguments $psi $ArgList

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $peakBytes = 0L
    $cpuMs = 0.0
    $gpuValues = New-Object System.Collections.Generic.List[double]
    $gpuScope = "unavailable"
    $stdout = ""
    $stderr = ""
    $exitCode = -1
    $nextGpuSampleMs = 0.0
    try {
        $process = [System.Diagnostics.Process]::Start($psi)
        $outTask = $process.StandardOutput.ReadToEndAsync()
        $errTask = $process.StandardError.ReadToEndAsync()
        while (-not $process.HasExited) {
            try {
                $process.Refresh()
                if ($process.PeakWorkingSet64 -gt $peakBytes) { $peakBytes = $process.PeakWorkingSet64 }
            } catch {}
            if ($sw.Elapsed.TotalMilliseconds -ge $nextGpuSampleMs) {
                $sample = Get-GpuUsageSample $process.Id
                if ($sample) {
                    $gpuValues.Add([double]$sample.value)
                    $gpuScope = $sample.scope
                }
                $nextGpuSampleMs = $sw.Elapsed.TotalMilliseconds + $GpuSampleIntervalMs
            }
            Start-Sleep -Milliseconds 25
        }
        try {
            $process.Refresh()
            if ($process.PeakWorkingSet64 -gt $peakBytes) { $peakBytes = $process.PeakWorkingSet64 }
        } catch {}
        $stdout = $outTask.Result
        $stderr = $errTask.Result
        $process.WaitForExit()
        $exitCode = $process.ExitCode
        try { $process.Refresh(); $cpuMs = $process.TotalProcessorTime.TotalMilliseconds } catch {}
    } catch {
        $stderr = $_.Exception.Message
    }
    $sw.Stop()

    $gpuPeak = $null
    if ($gpuValues.Count -gt 0) { $gpuPeak = ($gpuValues | Measure-Object -Maximum).Maximum }
    return [pscustomobject]@{
        stdout = $stdout; stderr = $stderr; exitCode = $exitCode
        elapsedMs = $sw.ElapsedMilliseconds; peakBytes = $peakBytes; cpuMs = $cpuMs
        gpuPeak = $gpuPeak; gpuScope = $gpuScope
    }
}

# PNG 的「覆盖谱面时长」= 整谱时长，取本次运行 render.log 汇总行的 chart_duration_ms。
function Get-ChartDurationSeconds {
    param([string]$Bid)

    if (-not (Test-Path -LiteralPath $renderLog)) { return $null }
    $result = $null
    foreach ($line in Get-Content -LiteralPath $renderLog) {
        if ($line -notlike "*`"bid`":`"$Bid`"*") { continue }
        if ($line -match '"chart_duration_ms":([0-9.]+)') {
            $result = [double]$Matches[1] / 1000.0
        }
    }
    return $result
}

# GIF 的「覆盖谱面时长」= 时间窗数 × 段长：网格容量恒为 4 段，
# 段长取该模式 gif 的 DURATION_MS（std/taiko/catch 5000ms，mania 10000ms）。
function Get-GifCoveredSeconds {
    param([string]$Mode)

    $segmentSeconds = if ($Mode -eq "mania") { 10.0 } else { 5.0 }
    return 4.0 * $segmentSeconds
}

function Format-Number {
    param($Value, [int]$Decimals)

    if ($null -eq $Value -or "$Value" -eq "") { return "-" }
    return ("{0:F$Decimals}" -f $Value)
}

# ── 谱面列表（备注取用户给的注释原文） ──
$maps = @(
    [pscustomobject]@{ mode = "std";   bid = "738063";  note = "No title 跳" }
    [pscustomobject]@{ mode = "std";   bid = "131564";  note = "折返滑条tech" }
    [pscustomobject]@{ mode = "std";   bid = "4378877"; note = "tech" }
    [pscustomobject]@{ mode = "std";   bid = "1259653"; note = "alt" }
    [pscustomobject]@{ mode = "std";   bid = "1024742"; note = "间距串" }
    [pscustomobject]@{ mode = "std";   bid = "4137259"; note = "串" }
    [pscustomobject]@{ mode = "std";   bid = "2824953"; note = "滑条变速多" }
    [pscustomobject]@{ mode = "std";   bid = "4763983"; note = "reading" }

    [pscustomobject]@{ mode = "taiko"; bid = "5108359"; note = "综合高难" }
    [pscustomobject]@{ mode = "taiko"; bid = "1491996"; note = "一般图" }
    [pscustomobject]@{ mode = "taiko"; bid = "5115616"; note = "sv" }
    [pscustomobject]@{ mode = "taiko"; bid = "5175577"; note = "sv" }
    [pscustomobject]@{ mode = "taiko"; bid = "4590051"; note = "变速爆发图" }

    [pscustomobject]@{ mode = "ctb";   bid = "3232957"; note = "一般图" }
    [pscustomobject]@{ mode = "ctb";   bid = "944502";  note = "No Dash" }
    [pscustomobject]@{ mode = "ctb";   bid = "5342747"; note = "一般图" }

    [pscustomobject]@{ mode = "mania"; bid = "5369780"; note = "7k大叠" }
    [pscustomobject]@{ mode = "mania"; bid = "3222380"; note = "4k中叠" }
    [pscustomobject]@{ mode = "mania"; bid = "5170802"; note = "4k rc" }
    [pscustomobject]@{ mode = "mania"; bid = "5416965"; note = "4k tb" }
    [pscustomobject]@{ mode = "mania"; bid = "4665942"; note = "6k ln" }
    [pscustomobject]@{ mode = "mania"; bid = "2473984"; note = "7k 夜曲" }
    [pscustomobject]@{ mode = "mania"; bid = "5156942"; note = "9k ln" }
    [pscustomobject]@{ mode = "mania"; bid = "4701252"; note = "16k" }
    [pscustomobject]@{ mode = "mania"; bid = "4789195"; note = "4k sv" }
    [pscustomobject]@{ mode = "mania"; bid = "3793380"; note = "4k sv" }
    [pscustomobject]@{ mode = "mania"; bid = "5221843"; note = "7k sv" }
)

function New-Task {
    param($Mode, $Bid, $Fmt, $Convert, $Mods, $Label, $Note, $CoveredSec)

    [pscustomobject]@{
        mode = $Mode; bid = $Bid; fmt = $Fmt
        convert = $Convert; mods = $Mods
        label = $Label; note = $Note
        coveredSec = $CoveredSec
    }
}

$tasks = New-Object System.Collections.Generic.List[object]
# 渲染顺序：每个模式先渲染完全部 png，再渲染 gif。
foreach ($modeName in @("std", "taiko", "ctb", "mania")) {
    $modeMaps = @($maps | Where-Object { $_.mode -eq $modeName })
    foreach ($m in $modeMaps) {
        $tasks.Add((New-Task $m.mode $m.bid "png" $null $null "-" $m.note $null))
    }
    foreach ($m in $modeMaps) {
        $tasks.Add((New-Task $m.mode $m.bid "gif" $null $null "-" $m.note (Get-GifCoveredSeconds $m.mode)))
    }
}

# ── 运行配置：输出目录、唯一日志目录；放宽超时避免长渲染被默认 300s 误杀 ──
$configJson = @{
    paths = @{ OUTPUT_DIR = $outdir; LOG_DIR = $logDir }
    timeout = @{ PNG_TIMEOUT = 3600; GIF_TIMEOUT = 3600; MP4_TIMEOUT = 3600 }
} | ConvertTo-Json -Compress -Depth 8

# ── 统一报告列（控制台与 report.txt 同列同序） ──
$columnFormat = "{0,3} {1,-6} {2,-9} {3,-4} {4,-22} {5,-6} {6,9} {7,9} {8,9} {9,10} {10,9} {11,7} {12,8}  {13}"
$columnTitles = "#", "MODE", "BID", "FMT", "任务", "STATUS", "渲染ms", "覆盖s", "每sms", "大小KB", "峰值MB", "CPU", "GPU峰值", "备注"

$results = New-Object System.Collections.Generic.List[object]
$totalCount = $tasks.Count
$index = 0

Write-Host ""
Write-Host ("=" * 130)
Write-Host "  一般测试集批量渲染：$totalCount 个任务（png + gif）"
Write-Host "  输出: $outdir"
Write-Host ("=" * 130)
Write-Host ($columnFormat -f $columnTitles)
Write-Host ("-" * 130)

foreach ($t in $tasks) {
    $index++

    $argList = @("--bid=$($t.bid)", "--fmt=$($t.fmt)", "--config=$configJson")
    if ($t.convert) { $argList += "--convert=$($t.convert)" }
    if ($t.mods) {
        foreach ($mod in $t.mods) { $argList += "--mod=$mod" }
    }

    $run = Invoke-Binary $argList

    $status = "ERR"
    $message = ""
    $output = ""
    $sizeBytes = 0L
    $json = $null
    if ($run.stdout -and $run.stdout.Trim().StartsWith("{")) {
        try { $json = $run.stdout | ConvertFrom-Json } catch { $json = $null }
    }
    if ($json) {
        $status = $json.status
        $message = $json.msg
        $sourcePath = $json.'preview-img'
        if ($status -eq "success" -and $sourcePath -and (Test-Path -LiteralPath $sourcePath -PathType Leaf)) {
            # 渲染器把产物写在配置哈希子目录里；复制到子文件夹根下平铺保存。
            $output = Split-Path $sourcePath -Leaf
            $flatPath = Join-Path $outdir $output
            Copy-Item -LiteralPath $sourcePath -Destination $flatPath -Force
            $sizeBytes = (Get-Item -LiteralPath $flatPath).Length
        }
    } else {
        $message = if ($run.stderr) { $run.stderr.Trim() } else { $run.stdout.Trim() }
    }

    $coveredSec = $t.coveredSec
    if ($t.fmt -eq "png") { $coveredSec = Get-ChartDurationSeconds $t.bid }
    $msPerSec = $null
    if ($coveredSec -and $coveredSec -gt 0) { $msPerSec = $run.elapsedMs / $coveredSec }
    $peakMB = $run.peakBytes / 1MB
    $sizeKB = $sizeBytes / 1KB
    $cpuPct = if ($run.elapsedMs -gt 0) { $run.cpuMs / $run.elapsedMs * 100.0 } else { 0.0 }

    $results.Add([pscustomobject]@{
        index = $index; mode = $t.mode; bid = $t.bid; fmt = $t.fmt; task = $t.label
        status = $status; renderMs = $run.elapsedMs; coveredSec = $coveredSec; msPerSec = $msPerSec
        sizeKB = $sizeKB; peakMB = $peakMB; cpuPct = $cpuPct
        gpuPeakPct = $run.gpuPeak; note = $t.note; output = $output
        args = ($argList -join " "); message = $message
    })

    Write-Host ($columnFormat -f $index, $t.mode, $t.bid, $t.fmt, $t.label, $status, `
        ("$($run.elapsedMs)"), (Format-Number $coveredSec 2), (Format-Number $msPerSec 2), `
        (Format-Number $sizeKB 1), (Format-Number $peakMB 1), ((Format-Number $cpuPct 1) + "%"), `
        $(if ($null -ne $run.gpuPeak) { (Format-Number $run.gpuPeak 1) + "%" } else { "-" }), $t.note)
}

# ── 汇总与报告 ──
$okResults = @($results | Where-Object { $_.status -eq "success" })
$failed = @($results | Where-Object { $_.status -ne "success" })
$totalMs = ($results | Measure-Object -Property renderMs -Sum).Sum
if ($null -eq $totalMs) { $totalMs = 0 }
$coveredTotal = ($okResults | Where-Object { $_.coveredSec } | Measure-Object -Property coveredSec -Sum).Sum
if ($null -eq $coveredTotal) { $coveredTotal = 0 }
$overallMsPerSec = if ($coveredTotal -gt 0) { $totalMs / $coveredTotal } else { $null }
$maxMem = ($results | Measure-Object -Property peakMB -Maximum).Maximum
$maxGpu = ($results | Where-Object { $null -ne $_.gpuPeakPct } | Measure-Object -Property gpuPeakPct -Maximum).Maximum
$avgCpu = if ($okResults.Count -gt 0) { ($okResults | Measure-Object -Property cpuPct -Average).Average } else { $null }
$maxCpu = if ($okResults.Count -gt 0) { ($okResults | Measure-Object -Property cpuPct -Maximum).Maximum } else { $null }
$now = Get-Date -Format "yyyy-MM-dd HH:mm:ss"

$lines = New-Object System.Collections.Generic.List[string]
$lines.Add("osu-beatmap-preview 批量渲染报告：一般测试集（PNG + GIF）")
$lines.Add("生成时间: $now")
$lines.Add("二进制: $bin")
$lines.Add("输出目录: $outdir")
$lines.Add("")
$lines.Add(("任务总数: {0}    成功: {1}    失败: {2}" -f $results.Count, $okResults.Count, $failed.Count))
$lines.Add(("总渲染时间: {0}ms ({1:F1}s)    总覆盖时长: {2:F1}s    总体每s渲染: {3}ms/s" -f `
    $totalMs, ($totalMs / 1000.0), $coveredTotal, (Format-Number $overallMsPerSec 2)))
$lines.Add(("峰值内存最大: {0}MB    GPU峰值最大: {1}%    CPU 平均: {2}%    最高: {3}%" -f `
    (Format-Number $maxMem 1), (Format-Number $maxGpu 1), (Format-Number $avgCpu 1), (Format-Number $maxCpu 1)))
$lines.Add("")
$lines.Add($columnFormat -f $columnTitles)
$lines.Add(("-" * 130))
foreach ($r in $results) {
    $lines.Add(($columnFormat -f $r.index, $r.mode, $r.bid, $r.fmt, $r.task, $r.status, `
        $(if ($null -ne $r.renderMs) { "$($r.renderMs)" } else { "-" }), `
        (Format-Number $r.coveredSec 2), (Format-Number $r.msPerSec 2), `
        (Format-Number $r.sizeKB 1), (Format-Number $r.peakMB 1), `
        $(if ($null -ne $r.cpuPct) { (Format-Number $r.cpuPct 1) + "%" } else { "-" }), `
        $(if ($null -ne $r.gpuPeakPct) { (Format-Number $r.gpuPeakPct 1) + "%" } else { "-" }), $r.note))
}
if ($failed.Count -gt 0) {
    $lines.Add("")
    $lines.Add("失败详情:")
    $lines.Add(("-" * 100))
    foreach ($r in $failed) {
        $lines.Add("[$($r.index)] $($r.mode) $($r.bid) $($r.fmt)  ($($r.args))")
        $lines.Add("    $($r.message)")
    }
}
$lines.Add("")
$lines.Add(("SUMMARY tasks={0} ok={1} fail={2} total_ms={3} covered_s={4}" -f `
    $results.Count, $okResults.Count, $failed.Count, [long]$totalMs, [math]::Round($coveredTotal, 3)))

$reportPath = Join-Path $outdir "report.txt"
$lines | Set-Content -LiteralPath $reportPath -Encoding UTF8

# results.csv 供总计脚本生成 report.md
$csvPath = Join-Path $outdir "results.csv"
$results | Select-Object `
    @{n = "index"; e = { $_.index } },
    @{n = "mode"; e = { $_.mode } },
    @{n = "bid"; e = { $_.bid } },
    @{n = "fmt"; e = { $_.fmt } },
    @{n = "task"; e = { $_.task } },
    @{n = "status"; e = { $_.status } },
    @{n = "render_ms"; e = { if ($null -ne $_.renderMs) { [math]::Round($_.renderMs, 1) } else { "" } } },
    @{n = "covered_s"; e = { if ($null -ne $_.coveredSec) { [math]::Round($_.coveredSec, 3) } else { "" } } },
    @{n = "ms_per_s"; e = { if ($null -ne $_.msPerSec) { [math]::Round($_.msPerSec, 2) } else { "" } } },
    @{n = "size_kb"; e = { if ($null -ne $_.sizeKB) { [math]::Round($_.sizeKB, 1) } else { "" } } },
    @{n = "peak_mb"; e = { if ($null -ne $_.peakMB) { [math]::Round($_.peakMB, 1) } else { "" } } },
    @{n = "cpu_pct"; e = { if ($null -ne $_.cpuPct) { [math]::Round($_.cpuPct, 1) } else { "" } } },
    @{n = "gpu_peak_pct"; e = { if ($null -ne $_.gpuPeakPct) { [math]::Round($_.gpuPeakPct, 1) } else { "" } } },
    @{n = "note"; e = { $_.note } },
    @{n = "output"; e = { $_.output } } |
    Export-Csv -LiteralPath $csvPath -NoTypeInformation -Encoding UTF8

# 清掉渲染器按配置哈希生成的中间目录：子文件夹里只留平铺产物、日志与报告。
Get-ChildItem -LiteralPath $outdir -Directory |
    Where-Object { $_.Name -ne ".logs" } |
    Remove-Item -Recurse -Force

# ── 渲染结束后清理 output 根目录散文件（不动文件夹），只保留对应子文件夹的文件 ──
Get-ChildItem -LiteralPath $outputsRoot -File -ErrorAction SilentlyContinue | Remove-Item -Force

Write-Host ("-" * 130)
Write-Host ("完成: 成功 {0}/{1}，失败 {2}，总渲染时间 {3:F1}s，总体每s {4}ms/s" -f `
    $okResults.Count, $results.Count, $failed.Count, ($totalMs / 1000.0), (Format-Number $overallMsPerSec 2))
Write-Host "产物输出: $outdir"
Write-Host "报告文件: $reportPath"

if ($failed.Count -gt 0) {
    exit 1
}
