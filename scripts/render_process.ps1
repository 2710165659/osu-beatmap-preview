# 批量渲染共用的进程计时与资源采样；GPU 查询在独立运行空间中执行。

function Resolve-Executable {
    param([string]$Value)

    if (Test-Path -LiteralPath $Value -PathType Leaf) {
        return (Resolve-Path -LiteralPath $Value).Path
    }
    $command = Get-Command $Value -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($command) { return $command.Source }
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

# 运行空间保留计数器失败状态；每次仅启动一个查询，避免慢查询堆积。
$script:gpuSampleScript = {
    param([int]$ProcessId, [string]$NvidiaSmi)

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

    if ($NvidiaSmi) {
        try {
            $values = @(
                & $NvidiaSmi --query-gpu=utilization.gpu `
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

# elapsedMs 为操作系统记录的进程存活时长，包含 CLI 启动、准备、编码和退出。
function Invoke-Binary {
    param([string[]]$ArgList, [switch]$CollectGpu)

    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $bin
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.StandardOutputEncoding = [System.Text.Encoding]::UTF8
    $psi.StandardErrorEncoding = [System.Text.Encoding]::UTF8
    $psi.UseShellExecute = $false
    $psi.WorkingDirectory = $repoRoot
    Add-ProcessArguments $psi $ArgList

    $gpuSampler = $null
    $gpuPending = $null
    $nvidiaSmi = $null
    if ($CollectGpu) {
        $nvidiaSmi = Resolve-Executable "nvidia-smi.exe"
        $gpuSampler = [System.Management.Automation.PowerShell]::Create()
    }
    $gpuValues = New-Object System.Collections.Generic.List[double]
    $gpuScope = "unavailable"
    $peakBytes = 0L
    $cpuMs = 0.0
    $stdout = ""
    $stderr = ""
    $exitCode = -1
    $elapsedMs = $null
    $process = $null
    $nextGpuSampleMs = 0.0
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    try {
        $process = [System.Diagnostics.Process]::Start($psi)
        $outTask = $process.StandardOutput.ReadToEndAsync()
        $errTask = $process.StandardError.ReadToEndAsync()
        if ($gpuSampler) {
            $gpuSampler.AddScript($script:gpuSampleScript.ToString()).AddArgument($process.Id).AddArgument($nvidiaSmi) | Out-Null
        }
        while (-not $process.HasExited) {
            try {
                $process.Refresh()
                if ($process.PeakWorkingSet64 -gt $peakBytes) { $peakBytes = $process.PeakWorkingSet64 }
            } catch {}
            if ($gpuPending -and $gpuPending.IsCompleted) {
                try {
                    foreach ($sample in $gpuSampler.EndInvoke($gpuPending)) {
                        if ($null -ne $sample) {
                            $gpuValues.Add([double]$sample.value)
                            $gpuScope = $sample.scope
                        }
                    }
                } catch {}
                $gpuPending = $null
                $nextGpuSampleMs = $sw.Elapsed.TotalMilliseconds + $GpuSampleIntervalMs
            }
            if ($gpuSampler -and -not $gpuPending -and $sw.Elapsed.TotalMilliseconds -ge $nextGpuSampleMs) {
                $gpuPending = $gpuSampler.BeginInvoke()
            }
            # 等待进程退出可立即唤醒，避免把固定轮询间隔加到短任务上。
            $process.WaitForExit(25) | Out-Null
        }
        $process.WaitForExit()
        $elapsedMs = ($process.ExitTime - $process.StartTime).TotalMilliseconds
        $exitCode = $process.ExitCode
        try {
            $process.Refresh()
            if ($process.PeakWorkingSet64 -gt $peakBytes) { $peakBytes = $process.PeakWorkingSet64 }
            $cpuMs = $process.TotalProcessorTime.TotalMilliseconds
        } catch {}
        $stdout = $outTask.Result
        $stderr = $errTask.Result
    } catch {
        $stderr = $_.Exception.Message
    } finally {
        $sw.Stop()
        if ($null -eq $elapsedMs) { $elapsedMs = $sw.Elapsed.TotalMilliseconds }
        # 查询可能比 CLI 更晚返回；停止采样、读取输出与释放资源都不扩大进程耗时。
        if ($gpuSampler) {
            try { $gpuSampler.Stop() } catch {}
            $gpuSampler.Dispose()
        }
        if ($process) { $process.Dispose() }
    }

    $gpuPeak = $null
    if ($gpuValues.Count -gt 0) { $gpuPeak = ($gpuValues | Measure-Object -Maximum).Maximum }
    return [pscustomobject]@{
        stdout = $stdout; stderr = $stderr; exitCode = $exitCode
        elapsedMs = [math]::Round($elapsedMs, 1); peakBytes = $peakBytes; cpuMs = $cpuMs
        gpuPeak = $gpuPeak; gpuScope = $gpuScope; gpuValues = $gpuValues
    }
}
