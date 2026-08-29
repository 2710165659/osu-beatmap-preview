# osu-beatmap-preview MP4 默认行为批量基准
#
# 用法：
#   powershell -File ".\batch_render_video.ps1"
#   powershell -File ".\batch_render_video.ps1" -NoCache
#
# 默认运行删除 13 个目标 MP4 及其旧版命名文件，复用 .osu 和 OSZ 缓存。
# -NoCache 会同时刷新输入缓存。ffprobe 用于读取完成视频的精确时长。

param(
    [switch]$NoCache,
    [ValidateRange(100, 5000)]
    [int]$GpuSampleIntervalMs = 500,
    [string]$FfprobePath = "ffprobe"
)

$ErrorActionPreference = "Continue"
$bin = Join-Path $PSScriptRoot "target\release\osu-beatmap-preview.exe"
$appOutputDir = [System.IO.Path]::GetFullPath(
    (Join-Path $env:TEMP "osu-beatmap-preview\outputs")
)
$outdir = Join-Path $appOutputDir "batch-video"
if (-not (Test-Path -LiteralPath $bin -PathType Leaf)) {
    throw "Release binary not found: $bin`nRun cargo build --release first."
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
    throw "ffprobe was not found. Install FFmpeg or pass -FfprobePath <path>."
}

New-Item -ItemType Directory -Force -Path $outdir | Out-Null
$logDir = Join-Path $outdir "logs"
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

function Get-ExpectedOutputName {
    param($Task)
    $prefix = switch ($Task.mode) {
        "std"   { "standard" }
        "taiko" { "taiko" }
        "ctb"   { "catch" }
        "mania" { "mania" }
        default { throw "Unknown mode: $($Task.mode)" }
    }
    return "${prefix}_$($Task.bid).mp4"
}

function Get-LegacyOutputName {
    param($Task)
    $prefix = switch ($Task.mode) {
        "std"   { "standard" }
        "taiko" { "taiko" }
        "ctb"   { "catch" }
        "mania" { "mania" }
        default { throw "Unknown mode: $($Task.mode)" }
    }
    return "${prefix}_$($Task.bid)_video-start0-duration600.mp4"
}

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

$results = New-Object System.Collections.Generic.List[object]
$totalCount = $tasks.Count
$index = 0

Write-Host ""
Write-Host ("=" * 150)
Write-Host "  Full MP4 benchmark: $totalCount beatmaps"
Write-Host "  Output: $outdir"
Write-Host "  GPU sampling interval: ${GpuSampleIntervalMs}ms"
Write-Host ("=" * 150)
$header = "{0,5} {1,-6} {2,-9} {3,-8} {4,9} {5,10} {6,10} {7,9} {8,9} {9,8} {10,9}" -f `
    "#", "MODE", "BID", "STATUS", "CHART(s)", "WALL(ms)", "ms/chart-s", "GPU AVG", "GPU PEAK", "MEM MB", "SIZE MB"
Write-Host $header
Write-Host ("-" * 150)

foreach ($task in $tasks) {
    $index++
    $expectedName = Get-ExpectedOutputName $task
    $expectedPath = [System.IO.Path]::GetFullPath((Join-Path $appOutputDir $expectedName))

    # 删除默认名称和旧版带时间后缀名称，强制重新渲染但保留输入缓存。
    if ((Split-Path -Parent $expectedPath) -ne $appOutputDir) {
        throw "Refusing to remove output outside the application output directory: $expectedPath"
    }
    $pathsToRemove = @($expectedPath)
    $legacyPath = [System.IO.Path]::GetFullPath((Join-Path $appOutputDir (Get-LegacyOutputName $task)))
    if ((Split-Path -Parent $legacyPath) -ne $appOutputDir) {
        throw "Refusing to remove output outside the application output directory: $legacyPath"
    }
    $pathsToRemove += $legacyPath
    foreach ($pathToRemove in $pathsToRemove) {
        if (Test-Path -LiteralPath $pathToRemove -PathType Leaf) {
            Remove-Item -LiteralPath $pathToRemove -Force
        }
    }

    $argList = @(
        "--bid=$($task.bid)",
        "--fmt=mp4"
    )
    if ($NoCache) { $argList += "--no-cache" }

    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $bin
    $psi.Arguments = ($argList -join " ")
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.StandardOutputEncoding = [System.Text.Encoding]::UTF8
    $psi.StandardErrorEncoding = [System.Text.Encoding]::UTF8
    $psi.UseShellExecute = $false
    $psi.WorkingDirectory = $PSScriptRoot

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $peakBytes = 0L
    $cpuMs = 0.0
    $gpuValues = New-Object System.Collections.Generic.List[double]
    $gpuScope = "unavailable"
    $stdout = ""
    $stderr = ""
    $exitCode = -1
    $processElapsedMs = 0.0

    try {
        $process = [System.Diagnostics.Process]::Start($psi)
        $outTask = $process.StandardOutput.ReadToEndAsync()
        $errTask = $process.StandardError.ReadToEndAsync()

        while (-not $process.HasExited) {
            try {
                $process.Refresh()
                if ($process.PeakWorkingSet64 -gt $peakBytes) { $peakBytes = $process.PeakWorkingSet64 }
            } catch {}

            $gpuSample = Get-GpuUsageSample $process.Id
            if ($gpuSample) {
                $gpuValues.Add([double]$gpuSample.value)
                $gpuScope = $gpuSample.scope
            }
            Start-Sleep -Milliseconds $GpuSampleIntervalMs
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
        try {
            $processElapsedMs = ($process.ExitTime - $process.StartTime).TotalMilliseconds
        } catch {}
    } catch {
        $stderr = $_.Exception.Message
    }
    $sw.Stop()

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
            $message = "ffprobe could not read a positive MP4 duration: $copiedPath"
        }
    } elseif ($exitCode -eq 0 -and -not $json) {
        $message = "Process exited successfully but did not return valid JSON."
    }

    # 进程时间戳不包含进程退出后可能残留的 GPU 轮询间隔；启动失败时回退到秒表计时。
    $elapsedMs = if ($processElapsedMs -gt 0) { $processElapsedMs } else { $sw.Elapsed.TotalMilliseconds }
    $msPerChartSecond = if ($durationSec -gt 0) { $elapsedMs / $durationSec } else { 0.0 }
    $peakMB = $peakBytes / 1MB
    $sizeMB = $sizeBytes / 1MB
    $cpuPct = if ($elapsedMs -gt 0) { $cpuMs / $elapsedMs * 100.0 } else { 0.0 }

    $gpuAvg = 0.0
    $gpuActiveAvg = 0.0
    $gpuPeak = 0.0
    if ($gpuValues.Count -gt 0) {
        $gpuAvg = [double](($gpuValues | Measure-Object -Average).Average)
        $gpuPeak = [double](($gpuValues | Measure-Object -Maximum).Maximum)
        $active = @($gpuValues | Where-Object { $_ -gt 0.5 })
        if ($active.Count -gt 0) {
            $gpuActiveAvg = [double](($active | Measure-Object -Average).Average)
        }
    }

    $result = [pscustomobject]@{
        index = $index
        mode = $task.mode
        bid = $task.bid
        status = $status
        durationSec = [math]::Round($durationSec, 3)
        elapsedMs = [math]::Round($elapsedMs, 1)
        msPerChartSecond = [math]::Round($msPerChartSecond, 2)
        gpuAvgPct = [math]::Round($gpuAvg, 1)
        gpuActiveAvgPct = [math]::Round($gpuActiveAvg, 1)
        gpuPeakPct = [math]::Round($gpuPeak, 1)
        gpuScope = $gpuScope
        gpuSamples = $gpuValues.Count
        cpuPct = [math]::Round($cpuPct, 1)
        peakMemoryMB = [math]::Round($peakMB, 1)
        sizeMB = [math]::Round($sizeMB, 2)
        output = $copiedPath
        args = ($argList -join " ")
        message = $message
    }
    $results.Add($result)

    $line = "{0}/{1} {2,-6} {3,-9} {4,-8} {5,9:F3} {6,10:F0} {7,10:F2} {8,8:F1}% {9,8:F1}% {10,8:F1} {11,9:F2}" -f `
        $index, $totalCount, $task.mode, $task.bid, $status, $durationSec, $elapsedMs,
        $msPerChartSecond, $gpuAvg, $gpuPeak, $peakMB, $sizeMB
    Write-Host $line
}

$reportPath = Join-Path $outdir "report.txt"
$okResults = @($results | Where-Object { $_.status -eq "success" -and $_.durationSec -gt 0 })
$totalElapsedMs = ($results | Measure-Object -Property elapsedMs -Sum).Sum
$successfulElapsedMs = ($okResults | Measure-Object -Property elapsedMs -Sum).Sum
$totalDurationSec = ($okResults | Measure-Object -Property durationSec -Sum).Sum
$overallMsPerSecond = if ($totalDurationSec -gt 0) { $successfulElapsedMs / $totalDurationSec } else { 0.0 }
$maxMemory = ($results | Measure-Object -Property peakMemoryMB -Maximum).Maximum
$maxGpu = ($results | Measure-Object -Property gpuPeakPct -Maximum).Maximum
$now = Get-Date -Format "yyyy-MM-dd HH:mm:ss"

$lines = New-Object System.Collections.Generic.List[string]
$lines.Add("osu-beatmap-preview full MP4 benchmark")
$lines.Add("Generated: $now")
$lines.Add("Binary: $bin")
$lines.Add("Output: $outdir")
$lines.Add("NoCache: $NoCache")
$lines.Add("GPU sampling interval: ${GpuSampleIntervalMs}ms")
$lines.Add("GPU scope: process = Windows per-process GPU Engine; system = nvidia-smi whole GPU")
$lines.Add("")
$lines.Add(("Tasks: {0}  Success: {1}  Failed: {2}" -f $results.Count, $okResults.Count, ($results.Count - $okResults.Count)))
$lines.Add(("Total chart/video duration: {0:F3}s" -f $totalDurationSec))
$lines.Add(("Total wall time: {0:F1}ms ({1:F2}s)" -f $totalElapsedMs, ($totalElapsedMs / 1000.0)))
$lines.Add(("Overall average render cost: {0:F2}ms per chart-second" -f $overallMsPerSecond))
$lines.Add(("Peak GPU: {0:F1}%  Peak process memory: {1:F1}MB" -f $maxGpu, $maxMemory))
$lines.Add("")
$lines.Add("GPU AVG includes download, audio preparation, rendering, and final mux wait.")
$lines.Add("GPU ACTIVE AVG excludes samples at or below 0.5%.")
$lines.Add("")
$lines.Add(("{0,3} {1,-6} {2,-9} {3,-8} {4,10} {5,11} {6,11} {7,9} {8,12} {9,10} {10,8} {11,9} {12,9}" -f `
    "#", "MODE", "BID", "STATUS", "CHART(s)", "WALL(ms)", "ms/chart-s", "GPU AVG", "GPU ACTIVE", "GPU PEAK", "CPU", "MEM MB", "SIZE MB"))
$lines.Add(("-" * 150))
foreach ($result in $results) {
    $lines.Add(("{0,3} {1,-6} {2,-9} {3,-8} {4,10:F3} {5,11:F1} {6,11:F2} {7,8:F1}% {8,11:F1}% {9,9:F1}% {10,7:F1}% {11,9:F1} {12,9:F2}" -f `
        $result.index, $result.mode, $result.bid, $result.status, $result.durationSec,
        $result.elapsedMs, $result.msPerChartSecond, $result.gpuAvgPct,
        $result.gpuActiveAvgPct, $result.gpuPeakPct, $result.cpuPct,
        $result.peakMemoryMB, $result.sizeMB))
}

$modeGroups = @($okResults | Group-Object mode)
if ($modeGroups.Count -gt 0) {
    $lines.Add("")
    $lines.Add("Per-mode summary:")
    foreach ($group in $modeGroups) {
        $modeDuration = ($group.Group | Measure-Object -Property durationSec -Sum).Sum
        $modeElapsed = ($group.Group | Measure-Object -Property elapsedMs -Sum).Sum
        $modeCost = if ($modeDuration -gt 0) { $modeElapsed / $modeDuration } else { 0.0 }
        $modeGpuAvg = ($group.Group | Measure-Object -Property gpuAvgPct -Average).Average
        $lines.Add(("  {0,-6} count={1,2} duration={2,9:F3}s wall={3,10:F1}ms cost={4,8:F2}ms/chart-s avgGPU={5,5:F1}%" -f `
            $group.Name, $group.Count, $modeDuration, $modeElapsed, $modeCost, $modeGpuAvg))
    }
}

$failed = @($results | Where-Object { $_.status -ne "success" })
if ($failed.Count -gt 0) {
    $lines.Add("")
    $lines.Add("Failures:")
    foreach ($result in $failed) {
        $lines.Add("[$($result.index)] $($result.mode) $($result.bid): $($result.message)")
        $lines.Add("  args: $($result.args)")
    }
}

$lines | Set-Content -LiteralPath $reportPath -Encoding UTF8

Write-Host ("-" * 150)
Write-Host ("Done: {0}/{1} successful, {2:F2}ms per chart-second, peak GPU {3:F1}%, peak memory {4:F1}MB" -f `
    $okResults.Count, $results.Count, $overallMsPerSecond, $maxGpu, $maxMemory)
Write-Host "Videos: $outdir"
Write-Host "Report: $reportPath"
