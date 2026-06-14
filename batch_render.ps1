# osu-beatmap-preview (Python) 批量渲染脚本
# 用法（可从任意目录运行）：  powershell -File "<path>\batch_render.ps1"
#
# 渲染内容：
#   - std / taiko / catch / mania 每张谱面都生成 png 与 gif
#   - 穿插 mod、转谱(convert)、指定时间点(time)、以及多特性组合示例
# 输出：
#   - 渲染图片复制到 %TEMP%\osu-beatmap-preview\outputs\batch-python
#   - 渲染完成后写入 report.txt，列出每张图的耗时与峰值内存

$ErrorActionPreference = "Continue"
$py    = "python"
$script = Join-Path $PSScriptRoot "run.py"
# 输出到 temp 下程序自身的输出目录：%TEMP%\osu-beatmap-preview\outputs\batch-python
$outdir = Join-Path $env:TEMP "osu-beatmap-preview\outputs\batch-python"

# ── 渲染前清空输出目录 ──
if (Test-Path $outdir) { Remove-Item "$outdir\*" -Recurse -Force }
New-Item -ItemType Directory -Force -Path $outdir | Out-Null

# ── 谱面列表 ──
$std   = @("738063", "2875069", "4897202", "1024742","372245", "1529760", "5467386")
$taiko = @("4242023", "1418246", "4590053", "2923535", "5651058", "3726150")
$catch = @("3852338", "3807626", "944502", "2571609", "265177")
$mania = @("4312004", "4610729", "5061439", "4789195", "3793380", "4665942", "5354177", "5221843", "5369780", "4972672", "5013742")

# 构造一个任务对象
function New-Task {
    param($mode, $bid, $fmt, $convert = $null, $mod = $null, $time = $null)
    [pscustomobject]@{
        mode = $mode; bid = $bid; fmt = $fmt
        convert = $convert; mod = $mod; time = $time
    }
}

$tasks = New-Object System.Collections.Generic.List[object]

# ── 基础任务：每张谱面 png + gif ──
foreach ($b in $std)   { $tasks.Add((New-Task "std"   $b "png")); $tasks.Add((New-Task "std"   $b "gif")) }
foreach ($b in $taiko) { $tasks.Add((New-Task "taiko" $b "png")); $tasks.Add((New-Task "taiko" $b "gif")) }
foreach ($b in $catch) { $tasks.Add((New-Task "catch" $b "png")); $tasks.Add((New-Task "catch" $b "gif")) }
foreach ($b in $mania) { $tasks.Add((New-Task "mania" $b "png")); $tasks.Add((New-Task "mania" $b "gif")) }

# ── 穿插：mod 示例（遵守各模式支持的 mod；DT/HT 仅 gif） ──
# std: 支持 EZ HR HD DA（gif/png），DT/HT 仅 gif
$tasks.Add((New-Task "std" "738063"  "gif" $null "hd+hr"))
$tasks.Add((New-Task "std" "2875069" "png" $null "hr"))
$tasks.Add((New-Task "std" "4897202" "gif" $null "dt1.3"))
$tasks.Add((New-Task "std" "1024742" "gif" $null "daar9.5+dacs4.5"))
$tasks.Add((New-Task "std" "5467386" "gif" $null "ez+hd"))
# taiko: 支持 EZ HR SW（png/gif）、CS（gif）、DT/HT（gif）
$tasks.Add((New-Task "taiko" "4242023" "gif" $null "hr"))
$tasks.Add((New-Task "taiko" "1418246" "gif" $null "dt"))
$tasks.Add((New-Task "taiko" "4590053" "png" $null "sw"))
$tasks.Add((New-Task "taiko" "2923535" "gif" $null "cs"))
# catch: 支持 EZ HR（png/gif）、DT/HT（gif）
$tasks.Add((New-Task "catch" "3852338" "gif" $null "hr"))
$tasks.Add((New-Task "catch" "3807626" "png" $null "ez"))
$tasks.Add((New-Task "catch" "944502" "gif" $null "dt1.4"))
# mania: 支持 IN HO（png/gif）、CS DS（gif）、K（key mod）
$tasks.Add((New-Task "mania" "4312004"  "png" $null "in"))
$tasks.Add((New-Task "mania" "4610729" "gif" $null "ho"))
$tasks.Add((New-Task "mania" "5061439" "gif" $null "cs"))
$tasks.Add((New-Task "mania" "4789195" "gif" $null "ds"))

# ── 穿插：转谱示例（仅 standard 谱面可转），png + gif ──
$tasks.Add((New-Task "convert" "738063"  "png" "taiko"))
$tasks.Add((New-Task "convert" "2875069"  "gif" "taiko"))
$tasks.Add((New-Task "convert" "4897202" "png" "ctb"))
$tasks.Add((New-Task "convert" "1024742" "gif" "ctb"))
$tasks.Add((New-Task "convert" "372245" "png" "mania"))
$tasks.Add((New-Task "convert" "1529760" "gif" "mania"))
$tasks.Add((New-Task "convert" "5467386" "gif" "taiko"))
$tasks.Add((New-Task "convert" "260177" "png" "taiko"))
$tasks.Add((New-Task "convert" "260177" "png" "ctb"))

# ── 穿插：指定时间点（仅 gif，最多 4 个，单位秒） ──
$tasks.Add((New-Task "std" "738063" "gif" $null $null "30+40+50+60"))
$tasks.Add((New-Task "std" "2875069" "gif" $null $null "10+25+60"))
$tasks.Add((New-Task "std" "4897202"  "gif" $null $null "45"))

# ── 穿插：多特性组合示例 ──
$tasks.Add((New-Task "convert" "738063"  "gif" "mania" "in"))           # 转谱 + mod
$tasks.Add((New-Task "convert" "2875069" "gif" "ctb"   "hr"))           # 转谱 + mod
$tasks.Add((New-Task "std"     "4897202" "gif" $null   "hd+dt1.25" "20+40"))  # mod + 时间点
$tasks.Add((New-Task "convert" "5467386"  "gif" "taiko" "hr" "15+30")) # 转谱 + mod + 时间点

# ── 为任务生成唯一标签（用于文件名） ──
function Get-Label {
    param($t)
    $label = $t.bid
    if ($t.convert) { $label += "_$($t.convert)" }
    if ($t.mod)     { $label += "_" + ($t.mod -replace '[^a-zA-Z0-9.]', '-') }
    if ($t.time)    { $label += "_t" + ($t.time -replace '\+', '-') }
    $label += "_$($t.fmt)"
    return $label
}

# ── 执行前统计 ──
$totalCount = $tasks.Count
$basicCount = ($tasks | Where-Object { -not $_.convert -and -not $_.mod -and -not $_.time }).Count
$modCount   = ($tasks | Where-Object { $_.mod -and -not $_.convert -and -not $_.time }).Count
$convCount  = ($tasks | Where-Object { $_.convert -and -not $_.mod -and -not $_.time }).Count
$timeCount  = ($tasks | Where-Object { $_.time -and -not $_.convert -and -not $_.mod }).Count
$comboCount = ($tasks | Where-Object { ($_.convert -and $_.mod) -or ($_.mod -and $_.time) -or ($_.convert -and $_.time) }).Count

Write-Host ""
Write-Host ("=" * 70)
Write-Host "  任务统计"
Write-Host ("-" * 70)
Write-Host ("  基础任务 (各模式 png+gif):          {0,4}" -f $basicCount)
Write-Host ("  Mod 示例 (HR/DT/EZ/HD/DA/IN/HO...):  {0,4}" -f $modCount)
Write-Host ("  转谱示例 (std->taiko/ctb/mania):     {0,4}" -f $convCount)
Write-Host ("  指定时间点 (--time):                 {0,4}" -f $timeCount)
Write-Host ("  多特性组合 (转谱+mod/时间):          {0,4}" -f $comboCount)
Write-Host ("-" * 70)
Write-Host ("  任务总计:                            {0,4}" -f $totalCount)
Write-Host ("=" * 70)
Write-Host ""

# ── 执行 ──
$results = New-Object System.Collections.Generic.List[object]
$index = 0

# 表头
$headerLine = "{0,6} {1,-7} {2,-40} {3,-9} {4,7}   {5,7}    {6,9}   {7,6}" -f `
    "#", "MODE", "LABEL", "STATUS", "TIME", "PEAKMEM", "SIZE", "%"
Write-Host $headerLine
Write-Host ("-" * 100)

foreach ($t in $tasks) {
    $index++
    $label = Get-Label $t

    # 组装命令行参数
    $argList = @($script, "--bid=$($t.bid)", "--fmt=$($t.fmt)")
    if ($t.convert) { $argList += "--convert=$($t.convert)" }
    if ($t.mod)     { $argList += "--mod=$($t.mod)" }
    if ($t.time)    { $argList += "--time=$($t.time)" }

    # 用 Process 启动以便采集峰值内存
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $py
    $psi.Arguments = ($argList -join " ")
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.StandardOutputEncoding = [System.Text.Encoding]::UTF8
    $psi.StandardErrorEncoding = [System.Text.Encoding]::UTF8
    $psi.UseShellExecute = $false
    $psi.WorkingDirectory = $PSScriptRoot

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $peakBytes = 0
    $cpuMs = 0
    $stdout = ""
    $stderr = ""
    $exitCode = -1
    try {
        $p = [System.Diagnostics.Process]::Start($psi)
        $outTask = $p.StandardOutput.ReadToEndAsync()
        $errTask = $p.StandardError.ReadToEndAsync()
        # 轮询峰值工作集（PeakWorkingSet64 由 OS 单调跟踪）
        while (-not $p.HasExited) {
            try { $p.Refresh(); if ($p.PeakWorkingSet64 -gt $peakBytes) { $peakBytes = $p.PeakWorkingSet64 } } catch {}
            Start-Sleep -Milliseconds 15
        }
        try { $p.Refresh(); if ($p.PeakWorkingSet64 -gt $peakBytes) { $peakBytes = $p.PeakWorkingSet64 } } catch {}
        $stdout = $outTask.Result
        $stderr = $errTask.Result
        $p.WaitForExit()
        $exitCode = $p.ExitCode
        # 采集 CPU 时间（内核 + 用户）
        try { $p.Refresh(); $cpuMs = $p.TotalProcessorTime.TotalMilliseconds } catch { $cpuMs = 0 }
    } catch {
        $stderr = $_.Exception.Message
    }
    $sw.Stop()

    # 以 JSON 里的 status 为准（程序成功/失败都会输出 JSON）；解析失败再看退出码
    $status = "ERR"
    $sizeBytes = 0
    $msg = ""
    $json = $null
    if ($stdout -and $stdout.Trim().StartsWith("{")) {
        try { $json = $stdout | ConvertFrom-Json } catch { $json = $null }
    }
    if ($json) {
        $status = $json.status
        $msg = $json.msg
        $src = $json.'preview-img'
        if ($status -eq "success" -and $src -and (Test-Path $src)) {
            $dest = "$outdir\$label.$($t.fmt)"
            Copy-Item $src $dest -Force
            $sizeBytes = (Get-Item $dest).Length
        }
    } else {
        $status = "ERR"
        $msg = if ($stderr) { $stderr } else { $stdout }
    }

    $elapsedMs = $sw.ElapsedMilliseconds
    $peakMB = [math]::Round($peakBytes / 1MB, 1)
    $sizeKB = [math]::Round($sizeBytes / 1KB, 1)
    # CPU 使用率 = CPU 时间 / 实际耗时（多核环境下可超过 100%）
    $cpuPct = if ($elapsedMs -gt 0) { [math]::Round($cpuMs / $elapsedMs * 100, 1) } else { 0 }

    $results.Add([pscustomobject]@{
        idx = $index; mode = $t.mode; label = $label
        args = ($argList -join " "); status = $status
        ms = $elapsedMs; peakMB = $peakMB; sizeKB = $sizeKB; cpuPct = $cpuPct; msg = $msg
    })

    $consoleLine = "{0}/{1} {2,-7} {3,-40} {4,-9} {5,7}ms  {6,7}MB  {7,9}KB  {8,5}%" -f `
        $index, $totalCount, $t.mode, $label, $status, $elapsedMs, $peakMB, $sizeKB, $cpuPct
    Write-Host $consoleLine
}

# ── 写入 txt 报告 ──
$reportPath = "$outdir\report.txt"
$totalMs = ($results | Measure-Object -Property ms -Sum).Sum
$maxMem  = ($results | Measure-Object -Property peakMB -Maximum).Maximum
$okCount = ($results | Where-Object { $_.status -eq "success" }).Count
$now = Get-Date -Format "yyyy-MM-dd HH:mm:ss"

# CPU 汇总统计
$okResults = $results | Where-Object { $_.status -eq "success" }
$avgCpu = if ($okResults.Count -gt 0) { [math]::Round(($okResults | Measure-Object -Property cpuPct -Average).Average, 1) } else { 0 }
$maxCpu = if ($okResults.Count -gt 0) { ($okResults | Measure-Object -Property cpuPct -Maximum).Maximum } else { 0 }
$totalCpuMs = ($okResults | ForEach-Object { $_.ms * $_.cpuPct / 100 } | Measure-Object -Sum).Sum

$lines = New-Object System.Collections.Generic.List[string]
$lines.Add("osu-beatmap-preview (Python) 批量渲染报告")
$lines.Add("生成时间: $now")
$lines.Add("任务总数: $($results.Count)    成功: $okCount    失败: $($results.Count - $okCount)")
$lines.Add(("总耗时: {0}ms ({1:F1}s)    峰值内存(单进程最大): {2}MB" -f $totalMs, ($totalMs / 1000), $maxMem))
$lines.Add(("CPU 统计: 平均 {0}%    最高 {1}%    总CPU时间 {2:F1}s" -f $avgCpu, $maxCpu, ($totalCpuMs / 1000)))
$lines.Add("")
$lines.Add(("{0,3}  {1,-7} {2,-42} {3,-9} {4,8}  {5,8}  {6,10}  {7,6}" -f "#", "MODE", "LABEL", "STATUS", "TIME", "PEAKMEM", "SIZE", "%"))
$lines.Add(("-" * 110))
foreach ($r in $results) {
    $lines.Add(("{0,3}  {1,-7} {2,-42} {3,-9} {4,6}ms  {5,6}MB  {6,8}KB  {7,5}%" -f `
        $r.idx, $r.mode, $r.label, $r.status, $r.ms, $r.peakMB, $r.sizeKB, $r.cpuPct))
}
# 失败任务的详细信息
$failed = $results | Where-Object { $_.status -ne "success" }
if ($failed.Count -gt 0) {
    $lines.Add("")
    $lines.Add("失败详情:")
    $lines.Add(("-" * 100))
    foreach ($r in $failed) {
        $lines.Add("[$($r.idx)] $($r.label)  ($($r.args))")
        $lines.Add("    $($r.msg)")
    }
}

$lines | Set-Content -Path $reportPath -Encoding UTF8

Write-Host ("-" * 100)
Write-Host ("完成: {0}/{1} 成功, 总耗时 {2:F1}s, 峰值内存 {3}MB, 平均CPU {4}%" -f $okCount, $results.Count, ($totalMs / 1000), $maxMem, $avgCpu)
Write-Host ("图片输出: $outdir")
Write-Host ("报告文件: $reportPath")
