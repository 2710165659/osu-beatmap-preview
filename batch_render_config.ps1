# osu-beatmap-preview 配置与缩放批量渲染测试
#
# 用法：
#   powershell -File ".\batch_render_config.ps1"
#
# 正式任务共 47 项：四模式无时间标签 GIF、1x1 GIF、三档 PNG/GIF/MP4，
# 以及 Mania 关闭 SV 标签的 PNG 与 30 FPS GIF/MP4。所有 MP4 从 preview
# 开始渲染 30 秒。首次运行会建立 OSZ 缓存，需要排除下载耗时时可再次运行。

param(
    [string]$FfprobePath = "ffprobe"
)

$ErrorActionPreference = "Continue"
$bin = Join-Path $PSScriptRoot "target\release\osu-beatmap-preview.exe"
$outdir = Join-Path $env:TEMP "osu-beatmap-preview\outputs\batch-config"

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
$runId = Get-Date -Format "yyyyMMdd-HHmmss-fff"

function New-ConfigTask {
    param(
        [string]$Name,
        [string]$Mode,
        [string]$ConfigMode,
        [string]$Bid,
        [string]$Format,
        [hashtable]$Overrides,
        [bool]$Video = $false
    )

    [pscustomobject]@{
        name = $Name
        mode = $Mode
        configMode = $ConfigMode
        bid = $Bid
        format = $Format
        overrides = $Overrides
        video = $Video
    }
}

$modeSpecs = @(
    [pscustomobject]@{ name = "standard"; mode = "standard"; configMode = "standard"; bid = "1024742" }
    [pscustomobject]@{ name = "taiko";    mode = "taiko";    configMode = "taiko";    bid = "5115616" }
    [pscustomobject]@{ name = "catch";    mode = "ctb";      configMode = "catch";    bid = "2103068" }
    [pscustomobject]@{ name = "mania";    mode = "mania";    configMode = "mania";    bid = "5572554" }
)

$tasks = New-Object System.Collections.Generic.List[object]
foreach ($spec in $modeSpecs) {
    $tasks.Add((New-ConfigTask `
        "$($spec.name)_gif_no_time" $spec.mode $spec.configMode $spec.bid "gif" `
        @{ SHOW_TIME_LABEL = $false }))

    $oneByOne = @{ SHOW_TIME_LABEL = $false }
    if ($spec.name -eq "taiko") {
        $oneByOne.ROW_COUNT = 1
    } elseif ($spec.name -eq "mania") {
        $oneByOne.IMAGES_PER_ROW = 1
    } else {
        $oneByOne.ROW_COUNT = 1
        $oneByOne.IMAGES_PER_ROW = 1
    }
    $tasks.Add((New-ConfigTask `
        "$($spec.name)_gif_no_time_1x1" $spec.mode $spec.configMode $spec.bid "gif" `
        $oneByOne))

    foreach ($format in @("png", "gif", "mp4")) {
        foreach ($scale in @(0.5, 1.0, 2.0)) {
            $tasks.Add((New-ConfigTask `
                ("{0}_{1}_{2}x" -f $spec.name, $format, $scale) `
                $spec.mode $spec.configMode $spec.bid $format `
                @{ SCALE = $scale } ($format -eq "mp4")))
        }
    }
}

$tasks.Add((New-ConfigTask `
    "mania_png_no_sv" "mania" "mania" "5572554" "png" `
    @{ SHOW_SV_LABEL = $false }))
$tasks.Add((New-ConfigTask `
    "mania_gif_no_sv_30fps" "mania" "mania" "5572554" "gif" `
    @{ SHOW_SV_LABEL = $false; FPS = 30 }))
$tasks.Add((New-ConfigTask `
    "mania_mp4_no_sv_30fps" "mania" "mania" "5572554" "mp4" `
    @{ SHOW_SV_LABEL = $false; FPS = 30 } $true))

function New-ConfigJson {
    param($Task, [string]$LogVariant)

    # 配置覆盖项必须按运行时 schema 分层，不能再写入已移除的 layout 节点。
    $formatConfig = @{}
    foreach ($entry in $Task.overrides.GetEnumerator()) {
        switch ($entry.Key) {
            "SCALE" {
                $formatConfig.SCALE = $entry.Value
            }
            { $_ -in @("ROW_COUNT", "IMAGES_PER_ROW") } {
                if (-not $formatConfig.ContainsKey("structure")) {
                    $formatConfig.structure = @{}
                }
                $formatConfig.structure[$entry.Key] = $entry.Value
            }
            default {
                if (-not $formatConfig.ContainsKey("style")) {
                    $formatConfig.style = @{}
                }
                $formatConfig.style[$entry.Key] = $entry.Value
            }
        }
    }

    $config = @{
        paths = @{
            OUTPUT_DIR = $outdir
            # LOG_DIR 不参与实际绘制；用唯一值生成新的缓存变体，避免命中旧成品。
            LOG_DIR = (Join-Path $outdir ".logs\$LogVariant")
        }
        render = @{
            $Task.configMode = @{
                $Task.format = $formatConfig
            }
        }
    }
    return ($config | ConvertTo-Json -Compress -Depth 12)
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

function Get-Resolution {
    param([string]$Path)

    $raw = & $ffprobe -v error -select_streams v:0 `
        -show_entries stream=width,height -of csv=s=x:p=0 $Path 2>$null
    if ($LASTEXITCODE -ne 0 -or -not $raw) { return "-" }
    return (($raw | Select-Object -First 1).Trim())
}

$results = New-Object System.Collections.Generic.List[object]
$totalCount = $tasks.Count
$index = 0

Write-Host ""
Write-Host ("=" * 125)
Write-Host "  Config render benchmark: $totalCount tasks"
Write-Host "  Output: $outdir"
Write-Host ("=" * 125)
$header = "{0,5} {1,-8} {2,-38} {3,-8} {4,-11} {5,9} {6,9} {7,10} {8,7}" -f `
    "#", "MODE", "LABEL", "STATUS", "RESOLUTION", "TIME", "PEAKMEM", "SIZE", "CPU"
Write-Host $header
Write-Host ("-" * 125)

foreach ($task in $tasks) {
    $index++
    $configJson = New-ConfigJson $task "$runId-$index"
    $argList = @(
        "--bid=$($task.bid)",
        "--convert=$($task.mode)",
        "--fmt=$($task.format)",
        "--config=$configJson",
        "--no-log"
    )
    if ($task.video) {
        $argList += "--time-points=preview"
        $argList += "--duration-time=30"
    }

    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $bin
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.StandardOutputEncoding = [System.Text.Encoding]::UTF8
    $psi.StandardErrorEncoding = [System.Text.Encoding]::UTF8
    $psi.UseShellExecute = $false
    $psi.WorkingDirectory = $PSScriptRoot
    Add-ProcessArguments $psi $argList

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $peakBytes = 0L
    $cpuMs = 0.0
    $stdout = ""
    $stderr = ""
    $exitCode = -1
    try {
        $process = [System.Diagnostics.Process]::Start($psi)
        $outTask = $process.StandardOutput.ReadToEndAsync()
        $errTask = $process.StandardError.ReadToEndAsync()
        while (-not $process.HasExited) {
            try {
                $process.Refresh()
                if ($process.PeakWorkingSet64 -gt $peakBytes) {
                    $peakBytes = $process.PeakWorkingSet64
                }
            } catch {}
            Start-Sleep -Milliseconds 15
        }
        $process.WaitForExit()
        try {
            $process.Refresh()
            if ($process.PeakWorkingSet64 -gt $peakBytes) {
                $peakBytes = $process.PeakWorkingSet64
            }
            $cpuMs = $process.TotalProcessorTime.TotalMilliseconds
        } catch {}
        $stdout = $outTask.Result
        $stderr = $errTask.Result
        $exitCode = $process.ExitCode
    } catch {
        $stderr = $_.Exception.Message
    }
    $sw.Stop()

    $status = "ERR"
    $message = ""
    $resolution = "-"
    $sizeBytes = 0L
    $flatPath = $null
    $json = $null
    if ($stdout -and $stdout.Trim().StartsWith("{")) {
        try { $json = $stdout | ConvertFrom-Json } catch {}
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
        $message = if ($stderr) { $stderr.Trim() } else { $stdout.Trim() }
    }

    $elapsedMs = $sw.ElapsedMilliseconds
    $peakMB = [math]::Round($peakBytes / 1MB, 1)
    $sizeKB = [math]::Round($sizeBytes / 1KB, 1)
    $cpuPct = if ($elapsedMs -gt 0) {
        [math]::Round($cpuMs / $elapsedMs * 100.0, 1)
    } else {
        0.0
    }
    $label = if ($flatPath) { Split-Path $flatPath -Leaf } else { $task.name }

    $results.Add([pscustomobject]@{
        index = $index
        mode = $task.configMode
        label = $label
        status = $status
        resolution = $resolution
        ms = $elapsedMs
        peakMB = $peakMB
        sizeKB = $sizeKB
        cpuPct = $cpuPct
        args = ($argList -join " ")
        message = $message
    })

    $line = "{0}/{1} {2,-8} {3,-38} {4,-8} {5,-11} {6,7}ms {7,7}MB {8,8}KB {9,6}%" -f `
        $index, $totalCount, $task.configMode, $label, $status, $resolution,
        $elapsedMs, $peakMB, $sizeKB, $cpuPct
    Write-Host $line
}

$reportPath = Get-FlatPath "report" ".txt"
$okResults = @($results | Where-Object { $_.status -eq "success" })
$totalMs = ($results | Measure-Object -Property ms -Sum).Sum
$maxMemory = ($results | Measure-Object -Property peakMB -Maximum).Maximum
$now = Get-Date -Format "yyyy-MM-dd HH:mm:ss"

$lines = New-Object System.Collections.Generic.List[string]
$lines.Add("osu-beatmap-preview configuration render report")
$lines.Add("Generated: $now")
$lines.Add("Output: $outdir")
$lines.Add(("Tasks: {0}  Success: {1}  Failed: {2}" -f `
    $results.Count, $okResults.Count, ($results.Count - $okResults.Count)))
$lines.Add(("Total measured time: {0}ms ({1:F3}s)  Peak memory: {2}MB" -f `
    $totalMs, ($totalMs / 1000.0), $maxMemory))
$lines.Add("")
$lines.Add(("{0,3} {1,-8} {2,-40} {3,-8} {4,-11} {5,9} {6,9} {7,10} {8,7}" -f `
    "#", "MODE", "LABEL", "STATUS", "RESOLUTION", "TIME", "PEAKMEM", "SIZE", "CPU"))
$lines.Add(("-" * 125))
foreach ($result in $results) {
    $lines.Add(("{0,3} {1,-8} {2,-40} {3,-8} {4,-11} {5,7}ms {6,7}MB {7,8}KB {8,6}%" -f `
        $result.index, $result.mode, $result.label, $result.status,
        $result.resolution, $result.ms, $result.peakMB, $result.sizeKB, $result.cpuPct))
}

$failed = @($results | Where-Object { $_.status -ne "success" })
if ($failed.Count -gt 0) {
    $lines.Add("")
    $lines.Add("Failures:")
    foreach ($result in $failed) {
        $lines.Add("[$($result.index)] $($result.label): $($result.message)")
        $lines.Add("  args: $($result.args)")
    }
}

$lines | Set-Content -LiteralPath $reportPath -Encoding UTF8

Write-Host ("-" * 125)
Write-Host ("Done: {0}/{1} successful, measured time {2:F3}s, peak memory {3}MB" -f `
    $okResults.Count, $results.Count, ($totalMs / 1000.0), $maxMemory)
Write-Host "Flattened outputs: $outdir"
Write-Host "Report: $reportPath"

if ($failed.Count -gt 0) {
    exit 1
}
