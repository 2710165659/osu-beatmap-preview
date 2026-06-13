$exe = (Resolve-Path ".\osu-beatmap-preview-rust\target\release\osu-beatmap-preview.exe").Path
$arg = "--bid=372245"
$times = @()
$mems = @()

for ($i = 1; $i -le 5; $i++) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $exe
    $psi.Arguments = $arg
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.CreateNoWindow = $true

    $proc = New-Object System.Diagnostics.Process
    $proc.StartInfo = $psi

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $proc.Start() | Out-Null

    # Spin-wait sampling — poll as fast as possible without sleep
    $peakWS = 0
    $peakPaged = 0
    $peakVM = 0
    $peakPrivate = 0
    $sampleCount = 0
    while (-not $proc.HasExited) {
        try {
            $proc.Refresh()
            $sampleCount++
            if ($proc.WorkingSet64 -gt $peakWS) { $peakWS = $proc.WorkingSet64 }
            if ($proc.PagedMemorySize64 -gt $peakPaged) { $peakPaged = $proc.PagedMemorySize64 }
            if ($proc.VirtualMemorySize64 -gt $peakVM) { $peakVM = $proc.VirtualMemorySize64 }
            if ($proc.PrivateMemorySize64 -gt $peakPrivate) { $peakPrivate = $proc.PrivateMemorySize64 }
        } catch { }
        # Micro-sleep to avoid 100% CPU, but keep it fast
        Start-Sleep -Milliseconds 1
    }

    $stdout = $proc.StandardOutput.ReadToEnd()
    $stderr = $proc.StandardError.ReadToEnd()
    $sw.Stop()

    $wsMB = [math]::Round($peakWS / 1MB, 2)
    $pagedMB = [math]::Round($peakPaged / 1MB, 2)
    $vmMB = [math]::Round($peakVM / 1MB, 2)
    $privMB = [math]::Round($peakPrivate / 1MB, 2)
    $elapsed = $sw.ElapsedMilliseconds
    $times += $elapsed
    $mems += $wsMB
    Write-Host "Run $i : ${elapsed}ms, WS=${wsMB}MB, Paged=${pagedMB}MB, VM=${vmMB}MB, Priv=${privMB}MB, samples=$sampleCount, Exit=$($proc.ExitCode)"
    $proc.Dispose()
}

$avgTime = ($times | Measure-Object -Average).Average
$avgMem = ($mems | Measure-Object -Average).Average
Write-Host ("--- Rust Average: {0:F0}ms, WS={1:F2}MB ---" -f $avgTime, $avgMem)
