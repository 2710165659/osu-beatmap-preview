# osu-beatmap-preview 批量渲染脚本 5：视频渲染（MP4 默认行为基准）
#
# 用法（可从任意目录运行）：
#   powershell -File "<repo>\scripts\batch_render_video.ps1"
#   powershell -File "<repo>\scripts\batch_render_video.ps1" -NoCache
#
# 渲染内容：
#   - 四模式共 13 张谱面，使用程序默认 MP4 区间行为（任务清单保持既有基准不变）
# 输出：
#   - 产物复制到默认输出目录的 batch-video 子文件夹：
#     %TEMP%\osu-beatmap-preview\outputs\batch-video
#   - report.txt / results.csv：统一报告列（含单项渲染时间、每s渲染时长、备注）与总耗时汇总
#   - 渲染结束后清理 outputs 根目录散文件（不动任何子文件夹），只保留对应子文件夹的文件
#
# 默认运行渲染前清空 batch-video 子文件夹，并用唯一日志目录生成新的配置哈希变体，
# 保证每次都全新渲染（输入的 .osu / OSZ 缓存照常复用）。
# -NoCache 会同时刷新输入缓存。ffprobe 用于读取完成视频的精确时长。

param(
    [switch]$NoCache,
    [ValidateRange(100, 5000)]
    [int]$GpuSampleIntervalMs = 500,
    [string]$FfprobePath = "ffprobe"
)

$ErrorActionPreference = "Continue"

# ── 路径：脚本位于 scripts/ 下，二进制与工作目录都在仓库根 ──
$repoRoot = Split-Path -Parent $PSScriptRoot
$bin = Join-Path $repoRoot "target\release\osu-beatmap-preview-cli.exe"
$outputsRoot = [System.IO.Path]::GetFullPath((Join-Path $env:TEMP "osu-beatmap-preview\outputs"))
$outdir = Join-Path $outputsRoot "batch-video"
$runId = Get-Date -Format "yyyyMMdd-HHmmss-fff"
# LOG_DIR 用每次运行的唯一值：既隔离缓存变体强制重渲染，又便于读取本次的 render.log。
$logDir = Join-Path $outdir ".logs\$runId"

if (-not (Test-Path -LiteralPath $bin -PathType Leaf)) {
    throw "未找到 Release 二进制：$bin`n请先运行 cargo build --release。"
}

function Resolve-Executable {
    param([string]$Value)

    if (Test-Path -LiteralPath $Value -PathType Leaf) {
        return (Resolve-Path -LiteralPath $Value).Path
    }
    $command = Get-Command $Value -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($command) { return $command.Source }
    return $null
}

$ffprobe = Resolve-Executable $FfprobePath
if (-not $ffprobe) {
    throw "未找到 ffprobe。请安装 FFmpeg 或传入 -FfprobePath <path>。"
}

# ── 渲染前清空自己的子文件夹：避免命中输出缓存导致计时失真 ──
if (Test-Path -LiteralPath $outdir) { Remove-Item -LiteralPath $outdir -Recurse -Force }
New-Item -ItemType Directory -Force -Path $outdir | Out-Null
New-Item -ItemType Directory -Force -Path $logDir | Out-Null

function New-VideoTask {
    param([string]$Mode, [string]$Bid)
    [pscustomobject]@{ mode = $Mode; bid = $Bid }
}

# 仅渲染原生模式，并使用程序的默认 MP4 区间行为。
$tasks = @(
    New-VideoTask "std"   "5242890"
    New-VideoTask "std"   "4897202"
    New-VideoTask "std"   "1024742"

    New-VideoTask "taiko" "5619629"
    New-VideoTask "taiko" "5175577"
    New-VideoTask "taiko" "1418246"

    New-VideoTask "ctb"   "944502"
    New-VideoTask "ctb"   "2103068"
    New-VideoTask "ctb"   "2182842"

    New-VideoTask "mania" "4624418"
    New-VideoTask "mania" "5572554"
    New-VideoTask "mania" "3562727"
    New-VideoTask "mania" "4312004"
)

function Get-Mp4DurationSeconds {
    param([string]$Path)

    $raw = & $ffprobe -v error -show_entries format=duration `
        -of "default=noprint_wrappers=1:nokey=1" $Path 2>$null
    if ($LASTEXITCODE -ne 0 -or -not $raw) { return 0.0 }
    $value = 0.0
    $ok = [double]::TryParse(
        (($raw | Select-Object -First 1).Trim()),
        [System.Globalization.NumberStyles]::Float,
        [System.Globalization.CultureInfo]::InvariantCulture,
        [ref]$value
    )
    if ($ok) { return $value }
    return 0.0
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
                # 任务管理器显示最繁忙的引擎，而不是把独立的 3D、复制和视频编码引擎相加到 100% 以上。
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

function Format-Number {
    param($Value, [int]$Decimals)

    if ($null -eq $Value -or "$Value" -eq "") { return "-" }
    return ("{0:F$Decimals}" -f $Value)
}

# ── 统一报告列（控制台与 report.txt 同列同序） ──
$columnFormat = "{0,3} {1,-6} {2,-9} {3,-4} {4,-22} {5,-6} {6,9} {7,9} {8,9} {9,10} {10,9} {11,7} {12,8}  {13}"
$columnTitles = "#", "MODE", "BID", "FMT", "任务", "STATUS", "渲染ms", "覆盖s", "每sms", "大小KB", "峰值MB", "CPU", "GPU峰值", "备注"

# 运行配置：输出目录指向本脚本子文件夹；唯一日志目录隔离缓存变体；放宽超时避免长渲染被默认 300s 误杀。
$configJson = @{
    paths = @{ OUTPUT_DIR = $outdir; LOG_DIR = $logDir }
    timeout = @{ PNG_TIMEOUT = 3600; GIF_TIMEOUT = 3600; MP4_TIMEOUT = 3600 }
} | ConvertTo-Json -Compress -Depth 8

$results = New-Object System.Collections.Generic.List[object]
$totalCount = $tasks.Count
$index = 0

Write-Host ""
Write-Host ("=" * 130)
Write-Host "  视频渲染批量基准：$totalCount 张谱面（MP4 默认区间行为）"
Write-Host "  输出: $outdir"
Write-Host "  GPU 采样间隔: ${GpuSampleIntervalMs}ms"
Write-Host ("=" * 130)
Write-Host ($columnFormat -f $columnTitles)
Write-Host ("-" * 130)

foreach ($task in $tasks) {
    $index++
    # 子文件夹已清空、日志目录每轮唯一（配置哈希变体不同），因此每个任务都会真正重新渲染。

    $argList = @(
        "--bid=$($task.bid)",
        "--fmt=mp4",
        "--config=$configJson"
    )
    if ($NoCache) { $argList += "--no-cache" }

    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $bin
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.StandardOutputEncoding = [System.Text.Encoding]::UTF8
    $psi.StandardErrorEncoding = [System.Text.Encoding]::UTF8
    $psi.UseShellExecute = $false
    $psi.WorkingDirectory = $repoRoot
    Add-ProcessArguments $psi $argList

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
                $gpuSample = Get-GpuUsageSample $process.Id
                if ($gpuSample) {
                    $gpuValues.Add([double]$gpuSample.value)
                    $gpuScope = $gpuSample.scope
                }
                $nextGpuSampleMs = $sw.Elapsed.TotalMilliseconds + $GpuSampleIntervalMs
            }
            Start-Sleep -Milliseconds 25
        }

        $process.WaitForExit()
        try {
            $process.Refresh()
            if ($process.PeakWorkingSet64 -gt $peakBytes) { $peakBytes = $process.PeakWorkingSet64 }
            $cpuMs = $process.TotalProcessorTime.TotalMilliseconds
        } catch {}
        $stdout = $outTask.Result
        $stderr = $errTask.Result
        $exitCode = $process.ExitCode
    } catch {
        $stderr = $_.Exception.Message
    }
    $sw.Stop()

    # 每个任务的原始 stdout/stderr 存档，便于事后排查。
    $stdoutPath = Join-Path $logDir "$($task.mode)_$($task.bid).stdout.txt"
    $stderrPath = Join-Path $logDir "$($task.mode)_$($task.bid).stderr.txt"
    $stdout | Set-Content -LiteralPath $stdoutPath -Encoding UTF8
    $stderr | Set-Content -LiteralPath $stderrPath -Encoding UTF8

    $status = "ERR"
    $message = ""
    $outputPath = $null
    $json = $null
    if ($stdout -and $stdout.Trim().StartsWith("{")) {
        try { $json = $stdout | ConvertFrom-Json } catch { $json = $null }
    }
    if ($json) {
        $status = $json.status
        $message = $json.msg
        $outputPath = $json.'preview-img'
    } else {
        $message = if ($stderr) { $stderr.Trim() } else { $stdout.Trim() }
    }

    $sizeBytes = 0L
    $durationSec = 0.0
    $copiedPath = $null
    if ($status -eq "success" -and $outputPath -and (Test-Path -LiteralPath $outputPath -PathType Leaf)) {
        $copiedPath = Join-Path $outdir (Split-Path $outputPath -Leaf)
        Copy-Item -LiteralPath $outputPath -Destination $copiedPath -Force
        $sizeBytes = (Get-Item -LiteralPath $copiedPath).Length
        $durationSec = Get-Mp4DurationSeconds $copiedPath
        if ($durationSec -le 0) {
            $status = "ERR"
            $message = "ffprobe 无法读取正的 MP4 时长: $copiedPath"
        }
    } elseif ($exitCode -eq 0 -and -not $json) {
        $message = "进程成功退出但未返回有效 JSON。"
    }

    $elapsedMs = $sw.Elapsed.TotalMilliseconds
    $msPerSec = if ($durationSec -gt 0) { $elapsedMs / $durationSec } else { $null }
    $peakMB = $peakBytes / 1MB
    $sizeKB = $sizeBytes / 1KB
    $cpuPct = if ($elapsedMs -gt 0) { $cpuMs / $elapsedMs * 100.0 } else { 0.0 }

    $gpuAvg = $null
    $gpuActiveAvg = $null
    $gpuPeak = $null
    if ($gpuValues.Count -gt 0) {
        $gpuAvg = ($gpuValues | Measure-Object -Average).Average
        $gpuPeak = ($gpuValues | Measure-Object -Maximum).Maximum
        $active = @($gpuValues | Where-Object { $_ -gt 0.5 })
        if ($active.Count -gt 0) {
            $gpuActiveAvg = ($active | Measure-Object -Average).Average
        }
    }

    $results.Add([pscustomobject]@{
        index = $index; mode = $task.mode; bid = $task.bid; fmt = "mp4"; task = "-"
        status = $status
        renderMs = [math]::Round($elapsedMs, 1); coveredSec = [math]::Round($durationSec, 3)
        msPerSec = if ($null -ne $msPerSec) { [math]::Round($msPerSec, 2) } else { $null }
        sizeKB = [math]::Round($sizeKB, 1); peakMB = [math]::Round($peakMB, 1)
        cpuPct = [math]::Round($cpuPct, 1)
        gpuPeakPct = if ($null -ne $gpuPeak) { [math]::Round($gpuPeak, 1) } else { $null }
        gpuAvgPct = if ($null -ne $gpuAvg) { [math]::Round($gpuAvg, 1) } else { $null }
        gpuActiveAvgPct = if ($null -ne $gpuActiveAvg) { [math]::Round($gpuActiveAvg, 1) } else { $null }
        gpuScope = $gpuScope; gpuSamples = $gpuValues.Count
        note = "-"; output = $copiedPath
        args = ($argList -join " "); message = $message
    })

    Write-Host ($columnFormat -f $index, $task.mode, $task.bid, "mp4", "-", $status, `
        ("{0:F0}" -f $elapsedMs), (Format-Number $durationSec 2), (Format-Number $msPerSec 2), `
        (Format-Number $sizeKB 1), (Format-Number $peakMB 1), ((Format-Number $cpuPct 1) + "%"), `
        $(if ($null -ne $gpuPeak) { (Format-Number $gpuPeak 1) + "%" } else { "-" }), "-")
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
$lines.Add("osu-beatmap-preview 批量渲染报告：视频渲染（MP4）")
$lines.Add("生成时间: $now")
$lines.Add("二进制: $bin")
$lines.Add("输出目录: $outdir")
$lines.Add("NoCache: $NoCache    GPU 采样间隔: ${GpuSampleIntervalMs}ms    GPU 口径: process = Windows 进程 GPU Engine；system = nvidia-smi 整卡")
$lines.Add("")
$lines.Add(("任务总数: {0}    成功: {1}    跳过: 0    失败: {2}" -f $results.Count, $okResults.Count, $failed.Count))
$lines.Add(("总渲染时间: {0:F1}ms ({1:F1}s)    总覆盖时长: {2:F1}s    总体每s渲染: {3}ms/s" -f `
    $totalMs, ($totalMs / 1000.0), $coveredTotal, (Format-Number $overallMsPerSec 2)))
$lines.Add(("峰值内存最大: {0}MB    GPU峰值最大: {1}%    CPU 平均: {2}%    最高: {3}%" -f `
    (Format-Number $maxMem 1), (Format-Number $maxGpu 1), (Format-Number $avgCpu 1), (Format-Number $maxCpu 1)))
$lines.Add("")
$lines.Add($columnFormat -f $columnTitles)
$lines.Add(("-" * 130))
foreach ($r in $results) {
    $lines.Add(($columnFormat -f $r.index, $r.mode, $r.bid, $r.fmt, $r.task, $r.status, `
        ("{0:F0}" -f $r.renderMs), (Format-Number $r.coveredSec 2), (Format-Number $r.msPerSec 2), `
        (Format-Number $r.sizeKB 1), (Format-Number $r.peakMB 1), ((Format-Number $r.cpuPct 1) + "%"), `
        $(if ($null -ne $r.gpuPeakPct) { (Format-Number $r.gpuPeakPct 1) + "%" } else { "-" }), $r.note))
}

# 视频脚本的附加统计：GPU 平均口径与分模式小结（不占统一报告列）。
$lines.Add("")
$lines.Add("GPU AVG 含下载、音频准备、渲染与封装等待；GPU ACTIVE AVG 排除 ≤0.5% 的采样点。")
$modeGroups = @($okResults | Group-Object mode)
if ($modeGroups.Count -gt 0) {
    $lines.Add("")
    $lines.Add("Per-mode summary:")
    foreach ($group in $modeGroups) {
        $modeDuration = ($group.Group | Measure-Object -Property coveredSec -Sum).Sum
        $modeElapsed = ($group.Group | Measure-Object -Property renderMs -Sum).Sum
        $modeCost = if ($modeDuration -gt 0) { $modeElapsed / $modeDuration } else { 0.0 }
        $modeGpuAvg = ($group.Group | Where-Object { $null -ne $_.gpuAvgPct } | Measure-Object -Property gpuAvgPct -Average).Average
        $lines.Add(("  {0,-6} count={1,2} duration={2,9:F3}s wall={3,10:F1}ms cost={4,8:F2}ms/s avgGPU={5,5:F1}%" -f `
            $group.Name, $group.Count, $modeDuration, $modeElapsed, $modeCost, (Format-Number $modeGpuAvg 1)))
    }
}

if ($failed.Count -gt 0) {
    $lines.Add("")
    $lines.Add("失败详情:")
    $lines.Add(("-" * 100))
    foreach ($r in $failed) {
        $lines.Add("[$($r.index)] $($r.mode) $($r.bid)  ($($r.args))")
        $lines.Add("    $($r.message)")
    }
}
$lines.Add("")
$lines.Add(("SUMMARY tasks={0} ok={1} skip=0 fail={2} total_ms={3} covered_s={4}" -f `
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
    @{n = "render_ms"; e = { $_.renderMs } },
    @{n = "covered_s"; e = { $_.coveredSec } },
    @{n = "ms_per_s"; e = { $_.msPerSec } },
    @{n = "size_kb"; e = { $_.sizeKB } },
    @{n = "peak_mb"; e = { $_.peakMB } },
    @{n = "cpu_pct"; e = { $_.cpuPct } },
    @{n = "gpu_peak_pct"; e = { $_.gpuPeakPct } },
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
Write-Host ("完成: 成功 {0}/{1}，失败 {2}，总渲染时间 {3:F1}s，总体每s {4}ms/s，峰值内存 {5}MB" -f `
    $okResults.Count, $results.Count, $failed.Count, ($totalMs / 1000.0), (Format-Number $overallMsPerSec 2), (Format-Number $maxMem 1))
Write-Host "视频输出: $outdir"
Write-Host "报告文件: $reportPath"

if ($failed.Count -gt 0) {
    exit 1
}
