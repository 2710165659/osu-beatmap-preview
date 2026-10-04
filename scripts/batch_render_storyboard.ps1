# osu-beatmap-preview 批量渲染脚本 7：背景视频 / 故事板（MP4 配置测试）
#
# 用法（可从任意目录运行）：
#   powershell -File "<repo>\scripts\batch_render_storyboard.ps1"
#
# 渲染内容：
#   - 背景视频组：开启 ENABLE_BACKGROUND_VIDEO 的 MP4 渲染
#   - 故事板组：开启 ENABLE_STORYBOARD（并同时开启背景视频）的 MP4 渲染，
#     个别任务叠加背景暗化配置
#   - 全部使用程序默认 MP4 区间行为，配置覆盖经 --config 注入（配置进哈希、命令行参数不进）
# 输出：
#   - 产物平铺重命名到默认输出目录的 batch-storyboard 子文件夹（冲突时加序号）：
#     %TEMP%\osu-beatmap-preview\outputs\batch-storyboard
#   - report.txt / results.csv：统一报告列（含单项渲染时间、每s渲染时长、备注）与总耗时汇总
#   - 渲染结束后清理 outputs 根目录散文件（不动任何子文件夹），只保留对应子文件夹的文件

param(
    [string]$FfprobePath = "ffprobe",
    [ValidateRange(100, 5000)]
    [int]$GpuSampleIntervalMs = 500
)

$ErrorActionPreference = "Continue"

# ── 路径：脚本位于 scripts/ 下，二进制与工作目录都在仓库根 ──
$repoRoot = Split-Path -Parent $PSScriptRoot
$bin = Join-Path $repoRoot "target\release\osu-beatmap-preview-cli.exe"
$outputsRoot = [System.IO.Path]::GetFullPath((Join-Path $env:TEMP "osu-beatmap-preview\outputs"))
$outdir = Join-Path $outputsRoot "batch-storyboard"
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

function Get-Resolution {
    param([string]$Path)

    $raw = & $ffprobe -v error -select_streams v:0 `
        -show_entries stream=width,height -of csv=s=x:p=0 $Path 2>$null
    if ($LASTEXITCODE -ne 0 -or -not $raw) { return "-" }
    return (($raw | Select-Object -First 1).Trim())
}

function Get-FlatPath {
    param([string]$BaseName, [string]$Extension)

    return (Join-Path $outdir ($BaseName + $Extension))
}

function Get-UniqueFlatPath {
    param([string]$BaseName, [string]$Extension)

    $candidate = Get-FlatPath $BaseName $Extension
    if (-not (Test-Path -LiteralPath $candidate)) {
        return $candidate
    }

    # 序号始终追加到原始名称，避免多次冲突后形成 name-1-2 这类链式名称。
    for ($suffix = 1; ; $suffix++) {
        $candidate = Get-FlatPath ("{0}-{1}" -f $BaseName, $suffix) $Extension
        if (-not (Test-Path -LiteralPath $candidate)) {
            return $candidate
        }
    }
}

function Format-Number {
    param($Value, [int]$Decimals)

    if ($null -eq $Value -or "$Value" -eq "") { return "-" }
    return ("{0:F$Decimals}" -f $Value)
}

function New-MediaTask {
    param(
        [string]$Name,
        [string]$Mode,
        [string]$ConfigMode,
        [string]$Bid,
        [string]$Kind,
        [string]$Note,
        [hashtable]$StyleOverrides,
        $Scale
    )

    [pscustomobject]@{
        name = $Name
        mode = $Mode
        configMode = $ConfigMode
        bid = $Bid
        kind = $Kind
        note = $Note
        style = $StyleOverrides
        scale = $Scale
    }
}

# ── 任务列表：kind 取用户给的分组注释，note 取用户给的注释原文 ──
# 背景视频组：ENABLE_BACKGROUND_VIDEO = true
$tasks = New-Object System.Collections.Generic.List[object]
$tasks.Add((New-MediaTask "bgvideo_std_5748062"   "std"   "standard" "5748062" "背景视频" "-" @{ ENABLE_BACKGROUND_VIDEO = $true } $null))
$tasks.Add((New-MediaTask "bgvideo_std_5725171"   "std"   "standard" "5725171" "背景视频" "-" @{ ENABLE_BACKGROUND_VIDEO = $true } $null))
$tasks.Add((New-MediaTask "bgvideo_taiko_5591138" "taiko" "taiko"    "5591138" "背景视频" "-" @{ ENABLE_BACKGROUND_VIDEO = $true } $null))
$tasks.Add((New-MediaTask "bgvideo_ctb_5815199"   "ctb"   "catch"    "5815199" "背景视频" "-" @{ ENABLE_BACKGROUND_VIDEO = $true } $null))
$tasks.Add((New-MediaTask "bgvideo_mania_5705679" "mania" "mania"    "5705679" "背景视频" "-" @{ ENABLE_BACKGROUND_VIDEO = $true } $null))

# 故事板组：ENABLE_STORYBOARD = true，同时开启背景视频（谱面可能自带视频）
$tasks.Add((New-MediaTask "storyboard_std_1006822" "std"   "standard" "1006822" "故事板" "故事板极端测试" `
    @{ ENABLE_STORYBOARD = $true; ENABLE_BACKGROUND_VIDEO = $true } $null))
$tasks.Add((New-MediaTask "storyboard_std_5839941" "std"   "standard" "5839941" "故事板" "故事板aspire；背景暗化0" `
    @{ ENABLE_STORYBOARD = $true; ENABLE_BACKGROUND_VIDEO = $true; BACKGROUND_DIM = 0.0 } $null))
$tasks.Add((New-MediaTask "storyboard_std_4621898" "std"   "standard" "4621898" "故事板" "故事板一般" `
    @{ ENABLE_STORYBOARD = $true; ENABLE_BACKGROUND_VIDEO = $true } $null))
$tasks.Add((New-MediaTask "storyboard_std_5238831" "std"   "standard" "5238831" "故事板" "故事板一般" `
    @{ ENABLE_STORYBOARD = $true; ENABLE_BACKGROUND_VIDEO = $true } $null))
$tasks.Add((New-MediaTask "storyboard_taiko_2315669" "taiko" "taiko" "2315669" "故事板" "-" `
    @{ ENABLE_STORYBOARD = $true; ENABLE_BACKGROUND_VIDEO = $true } $null))
$tasks.Add((New-MediaTask "storyboard_ctb_5778411" "ctb"   "catch"  "5778411" "故事板+视频" "故事板+视频" `
    @{ ENABLE_STORYBOARD = $true; ENABLE_BACKGROUND_VIDEO = $true } $null))
$tasks.Add((New-MediaTask "storyboard_mania_3928732" "mania" "mania" "3928732" "故事板" "故事板图；背景暗化0.3" `
    @{ ENABLE_STORYBOARD = $true; ENABLE_BACKGROUND_VIDEO = $true; BACKGROUND_DIM = 0.3 } $null))
$tasks.Add((New-MediaTask "storyboard_mania_4611014" "mania" "mania" "4611014" "故事板" "故事板图；背景暗化0.3" `
    @{ ENABLE_STORYBOARD = $true; ENABLE_BACKGROUND_VIDEO = $true; BACKGROUND_DIM = 0.3 } $null))
$tasks.Add((New-MediaTask "storyboard_mania_3970396" "mania" "mania" "3970396" "故事板" "故事板" `
    @{ ENABLE_STORYBOARD = $true; ENABLE_BACKGROUND_VIDEO = $true } $null))

function New-ConfigJson {
    param($Task, [string]$LogVariant)

    # 配置覆盖按运行时 schema 分层：SCALE 在格式层，其余在 style 层。
    $formatConfig = @{ style = @{} }
    foreach ($entry in $Task.style.GetEnumerator()) {
        $formatConfig.style[$entry.Key] = $entry.Value
    }
    if ($null -ne $Task.scale) {
        $formatConfig.SCALE = $Task.scale
    }

    $config = @{
        paths = @{
            OUTPUT_DIR = $outdir
            # LOG_DIR 不参与实际绘制；用唯一值生成新的缓存变体，避免命中旧成品。
            LOG_DIR = (Join-Path $outdir ".logs\$LogVariant")
        }
        # 放宽超时：极端/超长故事板渲染可能远超默认 300s 超时。
        timeout = @{ PNG_TIMEOUT = 3600; GIF_TIMEOUT = 3600; MP4_TIMEOUT = 3600 }
        render = @{
            $Task.configMode = @{
                mp4 = $formatConfig
            }
        }
    }
    return ($config | ConvertTo-Json -Compress -Depth 12)
}

# ── 统一报告列（控制台与 report.txt 同列同序） ──
$columnFormat = "{0,3} {1,-6} {2,-9} {3,-4} {4,-22} {5,-6} {6,9} {7,9} {8,9} {9,10} {10,9} {11,7} {12,8}  {13}"
$columnTitles = "#", "MODE", "BID", "FMT", "任务", "STATUS", "渲染ms", "覆盖s", "每sms", "大小KB", "峰值MB", "CPU", "GPU峰值", "备注"

$results = New-Object System.Collections.Generic.List[object]
$totalCount = $tasks.Count
$index = 0

Write-Host ""
Write-Host ("=" * 130)
Write-Host "  背景视频 / 故事板批量渲染：$totalCount 个任务（MP4）"
Write-Host "  输出: $outdir"
Write-Host ("=" * 130)
Write-Host ($columnFormat -f $columnTitles)
Write-Host ("-" * 130)

foreach ($task in $tasks) {
    $index++
    $configJson = New-ConfigJson $task "$runId-$index"
    $argList = @(
        "--bid=$($task.bid)",
        "--convert=$($task.mode)",
        "--fmt=mp4",
        "--config=$configJson"
    )

    $run = Invoke-Binary $argList

    $status = "ERR"
    $message = ""
    $resolution = "-"
    $sizeBytes = 0L
    $flatPath = $null
    $json = $null
    if ($run.stdout -and $run.stdout.Trim().StartsWith("{")) {
        try { $json = $run.stdout | ConvertFrom-Json } catch { $json = $null }
    }
    if ($json) {
        $status = $json.status
        $message = $json.msg
        $sourcePath = $json.'preview-img'
        if ($status -eq "success" -and $sourcePath -and
            (Test-Path -LiteralPath $sourcePath -PathType Leaf)) {
            $extension = [System.IO.Path]::GetExtension($sourcePath)
            $flatPath = Get-UniqueFlatPath $task.name $extension
            Copy-Item -LiteralPath $sourcePath -Destination $flatPath
            $sizeBytes = (Get-Item -LiteralPath $flatPath).Length
            $resolution = Get-Resolution $flatPath
        }
    } else {
        $message = if ($run.stderr) { $run.stderr.Trim() } else { $run.stdout.Trim() }
    }

    # 「覆盖谱面时长」= 输出视频时长。
    $coveredSec = $null
    if ($flatPath) { $coveredSec = Get-Mp4DurationSeconds $flatPath }
    $msPerSec = $null
    if ($coveredSec -and $coveredSec -gt 0) { $msPerSec = $run.elapsedMs / $coveredSec }
    $peakMB = $run.peakBytes / 1MB
    $sizeKB = $sizeBytes / 1KB
    $cpuPct = if ($run.elapsedMs -gt 0) { $run.cpuMs / $run.elapsedMs * 100.0 } else { 0.0 }

    # 备注 = 用户给的注释（渲染成功后附上分辨率）。
    $note = $task.note
    if ($resolution -ne "-") { $note = "$($task.note)（$resolution）" }

    $results.Add([pscustomobject]@{
        index = $index; mode = $task.mode; bid = $task.bid; fmt = "mp4"; task = $task.kind
        status = $status; renderMs = $run.elapsedMs; coveredSec = $coveredSec; msPerSec = $msPerSec
        sizeKB = $sizeKB; peakMB = $peakMB; cpuPct = $cpuPct
        gpuPeakPct = $run.gpuPeak; note = $note
        output = if ($flatPath) { Split-Path $flatPath -Leaf } else { "" }
        args = ($argList -join " "); message = $message
    })

    Write-Host ($columnFormat -f $index, $task.mode, $task.bid, "mp4", $task.kind, $status, `
        ("$($run.elapsedMs)"), (Format-Number $coveredSec 2), (Format-Number $msPerSec 2), `
        (Format-Number $sizeKB 1), (Format-Number $peakMB 1), ((Format-Number $cpuPct 1) + "%"), `
        $(if ($null -ne $run.gpuPeak) { (Format-Number $run.gpuPeak 1) + "%" } else { "-" }), $note)
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
$lines.Add("osu-beatmap-preview 批量渲染报告：背景视频 / 故事板")
$lines.Add("生成时间: $now")
$lines.Add("二进制: $bin")
$lines.Add("输出目录: $outdir")
$lines.Add("")
$lines.Add(("任务总数: {0}    成功: {1}    跳过: 0    失败: {2}" -f $results.Count, $okResults.Count, $failed.Count))
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
        $lines.Add("[$($r.index)] $($r.task) $($r.bid)  ($($r.args))")
        $lines.Add("    $($r.message)")
    }
}
$lines.Add("")
$lines.Add(("SUMMARY tasks={0} ok={1} skip=0 fail={2} total_ms={3} covered_s={4}" -f `
    $results.Count, $okResults.Count, $failed.Count, [long]$totalMs, [math]::Round($coveredTotal, 3)))

$reportPath = Get-FlatPath "report" ".txt"
$lines | Set-Content -LiteralPath $reportPath -Encoding UTF8

# results.csv 供总计脚本生成 report.md
$csvPath = Get-FlatPath "results" ".csv"
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
