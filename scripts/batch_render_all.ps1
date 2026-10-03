# osu-beatmap-preview 批量渲染脚本 8：一键总计
#
# 用法（可从任意目录运行）：
#   powershell -File "<repo>\scripts\batch_render_all.ps1"
#   powershell -File "<repo>\scripts\batch_render_all.ps1" -NoCache
#   powershell -File "<repo>\scripts\batch_render_all.ps1" -UpdateDocs
#
# 渲染内容：
#   - 顺序执行 7 个批量渲染脚本（一般测试集、极端测试谱面、mod 测试、转谱、
#     视频渲染、配置渲染、背景视频/故事板），逐个记录退出码与墙钟耗时
# 输出（默认输出目录的 batch-all 子文件夹）：
#   - report.txt：各脚本任务数/成功/跳过/失败/渲染总耗时与全部脚本的合计
#   - report.md：包含全部单项结果、按格式/模式的汇总与总结，
#     可直接替换 docs\report.md（或使用 -UpdateDocs 自动同步）
#   - 任一脚本失败时最终退出码为 1

param(
    [string]$FfprobePath = "ffprobe",
    [switch]$NoCache,
    [switch]$UpdateDocs
)

$ErrorActionPreference = "Continue"

# ── 路径：脚本位于 scripts/ 下，二进制与工作目录都在仓库根 ──
$repoRoot = Split-Path -Parent $PSScriptRoot
$scriptsDir = $PSScriptRoot
$bin = Join-Path $repoRoot "target\release\osu-beatmap-preview-cli.exe"
$outputsRoot = [System.IO.Path]::GetFullPath((Join-Path $env:TEMP "osu-beatmap-preview\outputs"))
$outdir = Join-Path $outputsRoot "batch-all"
New-Item -ItemType Directory -Force -Path $outdir | Out-Null

# 子脚本用与当前宿主相同的 PowerShell 运行，避免 pwsh / Windows PowerShell 混用。
$hostExe = (Get-Process -Id $PID).Path

$scriptDefs = @(
    [pscustomobject]@{
        file = "batch_render_general.ps1"; sub = "batch-general"; title = "一般测试集（PNG + GIF）"
        desc = "四个模式的一般测试谱面，每张渲染 png 与 gif 各一次，覆盖常规滑条、串、变速与 reading 等典型场景。"
    }
    [pscustomobject]@{
        file = "batch_render_extreme.ps1"; sub = "batch-extreme"; title = "极端测试谱面（PNG + GIF）"
        desc = "极端谱面压力测试：超多滑条控制点、aspire、几万 BPM、超高密度与 vibro 等。"
    }
    [pscustomobject]@{
        file = "batch_render_mods.ps1"; sub = "batch-mods"; title = "mod 测试（GIF + PNG/跳过）"
        desc = "四个模式的 mod 组合渲染；gif 全量渲染，png 在该模式不支持对应 mod 时跳过（状态记 SKIP）。"
    }
    [pscustomobject]@{
        file = "batch_render_convert.ps1"; sub = "batch-convert"; title = "转谱测试（PNG + GIF）"
        desc = "std 谱面转 taiko / ctb / mania，含 mania 键数（1K~10K）与 DS mod 组合。"
    }
    [pscustomobject]@{
        file = "batch_render_video.ps1"; sub = "batch-video"; title = "视频渲染（MP4）"
        desc = "四模式共 13 张谱面，使用程序默认 MP4 区间行为，统计渲染耗时、每s渲染时长与资源占用。"
    }
    [pscustomobject]@{
        file = "batch_render_config.ps1"; sub = "batch-config"; title = "配置渲染"
        desc = "47 项配置组合：无时间标签 GIF、1x1 GIF、三档缩放 PNG/GIF/MP4、关闭 SV 标签与 30FPS 等。"
    }
    [pscustomobject]@{
        file = "batch_render_storyboard.ps1"; sub = "batch-storyboard"; title = "背景视频 / 故事板"
        desc = "开启背景视频与故事板的 MP4 渲染，含背景暗化、缩放、帧率与 mania 轨道暗化等配置覆盖。"
    }
)

# ── 逐个执行子脚本并记录墙钟与退出码 ──
$runs = New-Object System.Collections.Generic.List[object]
foreach ($def in $scriptDefs) {
    $scriptPath = Join-Path $scriptsDir $def.file
    if (-not (Test-Path -LiteralPath $scriptPath -PathType Leaf)) {
        throw "缺少子脚本: $scriptPath"
    }

    Write-Host ""
    Write-Host ("#" * 130)
    Write-Host ("# 执行 {0}（{1}）", $def.file, $def.title)
    Write-Host ("#" * 130)

    $argumentList = @("-NoProfile", "-File", $scriptPath, "-FfprobePath", $FfprobePath)
    if ($def.file -eq "batch_render_video.ps1" -and $NoCache) {
        $argumentList += "-NoCache"
    }
    # 前 4 个 PNG/GIF 脚本不接受 -FfprobePath（不依赖 ffprobe）。
    if ($def.file -in @("batch_render_general.ps1", "batch_render_extreme.ps1", `
            "batch_render_mods.ps1", "batch_render_convert.ps1")) {
        $argumentList = @("-NoProfile", "-File", $scriptPath)
    }

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    & $hostExe @argumentList
    $exitCode = $LASTEXITCODE
    $sw.Stop()

    $runs.Add([pscustomobject]@{
        def = $def
        exitCode = $exitCode
        wallMs = $sw.Elapsed.TotalMilliseconds
    })
}

# ── 汇总各脚本的 results.csv ──
function Get-NumberOrNull {
    param($Value)

    if ($null -eq $Value) { return $null }
    $text = "$Value".Trim()
    if ($text -eq "") { return $null }
    $parsed = 0.0
    if ([double]::TryParse($text, [System.Globalization.NumberStyles]::Float, `
            [System.Globalization.CultureInfo]::InvariantCulture, [ref]$parsed)) {
        return $parsed
    }
    return $null
}

function Format-Number {
    param($Value, [int]$Decimals)

    if ($null -eq $Value -or "$Value" -eq "") { return "-" }
    return ("{0:F$Decimals}" -f $Value)
}

# Markdown 单元格转义：竖线与换行会破坏表格结构。
function Escape-MarkdownCell {
    param($Value)

    $text = if ($null -eq $Value) { "" } else { "$Value" }
    return $text.Replace("\", "\\").Replace("|", "\|").Replace("`r`n", " ").Replace("`n", " ")
}

$sections = New-Object System.Collections.Generic.List[object]
$allRows = New-Object System.Collections.Generic.List[object]

foreach ($run in $runs) {
    $csvPath = Join-Path $outputsRoot "$($run.def.sub)\results.csv"
    $rows = @()
    if (Test-Path -LiteralPath $csvPath -PathType Leaf) {
        $rows = @(Import-Csv -LiteralPath $csvPath)
    }

    $ok = @($rows | Where-Object { $_.status -eq "success" }).Count
    $skip = @($rows | Where-Object { $_.status -eq "SKIP" }).Count
    $fail = @($rows | Where-Object { $_.status -ne "success" -and $_.status -ne "SKIP" }).Count
    $totalMs = 0.0
    $coveredSec = 0.0
    $maxMem = 0.0
    foreach ($row in $rows) {
        $ms = Get-NumberOrNull $row.render_ms
        if ($null -ne $ms) { $totalMs += $ms }
        $sec = Get-NumberOrNull $row.covered_s
        if ($null -ne $sec -and $row.status -eq "success") { $coveredSec += $sec }
        $mem = Get-NumberOrNull $row.peak_mb
        if ($null -ne $mem -and $mem -gt $maxMem) { $maxMem = $mem }
    }
    $msPerSec = if ($coveredSec -gt 0) { $totalMs / $coveredSec } else { $null }

    $sections.Add([pscustomobject]@{
        run = $run
        rows = $rows
        ok = $ok; skip = $skip; fail = $fail
        totalMs = $totalMs; coveredSec = $coveredSec
        msPerSec = $msPerSec; maxMem = $maxMem
    })
    foreach ($row in $rows) {
        $allRows.Add([pscustomobject]@{
            script = $run.def.title
            row = $row
        })
    }
}

$grandTasks = ($sections | ForEach-Object { $_.rows.Count } | Measure-Object -Sum).Sum
if ($null -eq $grandTasks) { $grandTasks = 0 }
$grandOk = ($sections | Measure-Object -Property ok -Sum).Sum
$grandSkip = ($sections | Measure-Object -Property skip -Sum).Sum
$grandFail = ($sections | Measure-Object -Property fail -Sum).Sum
$grandMs = ($sections | Measure-Object -Property totalMs -Sum).Sum
$grandCovered = ($sections | Measure-Object -Property coveredSec -Sum).Sum
$grandMsPerSec = if ($grandCovered -gt 0) { $grandMs / $grandCovered } else { $null }
$grandWallMs = ($runs | Measure-Object -Property wallMs -Sum).Sum
$grandMaxMem = ($sections | Measure-Object -property maxMem -Maximum).Maximum
$now = Get-Date -Format "yyyy-MM-dd HH:mm:ss"

# ── report.txt：全部脚本的大致汇总 ──
$txtLines = New-Object System.Collections.Generic.List[string]
$txtLines.Add("osu-beatmap-preview 批量渲染总计报告")
$txtLines.Add("生成时间: $now")
$txtLines.Add("输出根目录: $outputsRoot")
$txtLines.Add("")
$txtLines.Add(("{0,-28} {1,6} {2,6} {3,6} {4,6} {10,10} {5,12} {6,12} {7,12} {8,10} {9,8}" -f `
    "脚本", "任务", "成功", "跳过", "失败", "渲染ms", "覆盖s", "每sms", "峰值MB", "退出码", "墙钟s"))
$txtLines.Add(("-" * 130))
foreach ($section in $sections) {
    $txtLines.Add(("{0,-28} {1,6} {2,6} {3,6} {4,6} {10,10} {5,12} {6,12} {7,12} {8,10} {9,8}" -f `
        $section.run.def.file, $section.rows.Count, $section.ok, $section.skip, $section.fail, `
        [math]::Round($section.totalMs, 1), (Format-Number $section.coveredSec 1), `
        (Format-Number $section.msPerSec 2), (Format-Number $section.maxMem 1), `
        $section.run.exitCode, (Format-Number ($section.run.wallMs / 1000.0) 1)))
}
$txtLines.Add(("-" * 130))
$txtLines.Add(("全部合计: 任务 {0}  成功 {1}  跳过 {2}  失败 {3}" -f $grandTasks, $grandOk, $grandSkip, $grandFail))
$txtLines.Add(("总渲染时间: {0:F1}ms ({1:F1}s)    总覆盖时长: {2:F1}s    总体每s渲染: {3}ms/s" -f `
    $grandMs, ($grandMs / 1000.0), $grandCovered, (Format-Number $grandMsPerSec 2)))
$txtLines.Add(("全部脚本墙钟: {0:F1}s    峰值内存最大: {1}MB" -f ($grandWallMs / 1000.0), (Format-Number $grandMaxMem 1)))
$txtLines.Add("")
$txtLines.Add(("SUMMARY tasks={0} ok={1} skip={2} fail={3} total_ms={4} covered_s={5}" -f `
    $grandTasks, $grandOk, $grandSkip, $grandFail, [long]$grandMs, [math]::Round($grandCovered, 3)))
$txtPath = Join-Path $outdir "report.txt"
$txtLines | Set-Content -LiteralPath $txtPath -Encoding UTF8

# ── report.md：全部单项 + 汇总 + 总结（可直接替换 docs\report.md） ──
$md = New-Object System.Collections.Generic.List[string]
$md.Add("# 批量渲染报告")
$md.Add("")
$md.Add("本文件由 ``scripts/batch_render_all.ps1`` 自动生成，汇总七类批量渲染任务的全部单项结果、按格式与模式的汇总以及总体总结。生成时间：$now。")
$md.Add("")
$md.Add("- 渲染器：``$bin``")
$md.Add("- 输出根目录：``$outputsRoot``")
$md.Add("- 报告列说明：``渲染ms`` 为单项渲染墙钟时间；``覆盖s`` 为该任务覆盖的谱面时长（GIF 按各时间窗合计、MP4 按输出视频时长、PNG 按整谱时长）；``每sms`` = 渲染ms ÷ 覆盖s；GPU 采样不到时记 ``-``。")
$md.Add("")

# 总览
$md.Add("## 总览")
$md.Add("")
$md.Add("| 脚本 | 任务 | 成功 | 跳过 | 失败 | 总渲染时间(s) | 总覆盖时长(s) | 总体每s(ms/s) | 峰值内存(MB) | 墙钟(s) |")
$md.Add("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |")
foreach ($section in $sections) {
    $md.Add(("| {0} | {1} | {2} | {3} | {4} | {5:F1} | {6} | {7} | {8} | {9} |" -f `
        (Escape-MarkdownCell $section.run.def.title), $section.rows.Count, $section.ok, $section.skip, $section.fail, `
        ($section.totalMs / 1000.0), (Format-Number $section.coveredSec 1), `
        (Format-Number $section.msPerSec 2), (Format-Number $section.maxMem 1), `
        (Format-Number ($section.run.wallMs / 1000.0) 1)))
}
$md.Add(("| **全部合计** | **{0}** | **{1}** | **{2}** | **{3}** | **{4:F1}** | **{5}** | **{6}** | **{7}** | **{8}** |" -f `
    $grandTasks, $grandOk, $grandSkip, $grandFail, ($grandMs / 1000.0), `
    (Format-Number $grandCovered 1), (Format-Number $grandMsPerSec 2), (Format-Number $grandMaxMem 1), `
    (Format-Number ($grandWallMs / 1000.0) 1)))
$md.Add("")

# 全部单项
$rowHeader = "| # | 模式 | BID | 格式 | 任务 | 状态 | 渲染(ms) | 覆盖(s) | 每s(ms) | 大小(KB) | 峰值(MB) | CPU | GPU峰值 | 备注 |"
$rowDivider = "| ---: | --- | ---: | --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |"
foreach ($section in $sections) {
    $md.Add("## $($section.run.def.title)")
    $md.Add("")
    $md.Add($section.run.def.desc)
    $md.Add("")
    $md.Add($rowHeader)
    $md.Add($rowDivider)
    foreach ($row in $section.rows) {
        $renderMs = Get-NumberOrNull $row.render_ms
        $covered = Get-NumberOrNull $row.covered_s
        $msPerSec = Get-NumberOrNull $row.ms_per_s
        $sizeKb = Get-NumberOrNull $row.size_kb
        $peakMb = Get-NumberOrNull $row.peak_mb
        $cpuPct = Get-NumberOrNull $row.cpu_pct
        $gpuPeak = Get-NumberOrNull $row.gpu_peak_pct
        $md.Add(("| {0} | {1} | {2} | {3} | {4} | {5} | {6} | {7} | {8} | {9} | {10} | {11} | {12} | {13} |" -f `
            (Escape-MarkdownCell $row.index), (Escape-MarkdownCell $row.mode), (Escape-MarkdownCell $row.bid), `
            (Escape-MarkdownCell $row.fmt), (Escape-MarkdownCell $row.task), (Escape-MarkdownCell $row.status), `
            $(if ($null -ne $renderMs) { Format-Number $renderMs 1 } else { "-" }), `
            (Format-Number $covered 2), (Format-Number $msPerSec 2), (Format-Number $sizeKb 1), `
            (Format-Number $peakMb 1), `
            $(if ($null -ne $cpuPct) { (Format-Number $cpuPct 1) + "%" } else { "-" }), `
            $(if ($null -ne $gpuPeak) { (Format-Number $gpuPeak 1) + "%" } else { "-" }), `
            (Escape-MarkdownCell $row.note)))
    }
    $md.Add("")
}

# 汇总：按输出格式 / 按游戏模式
function Add-GroupSummary {
    param($Lines, [string]$Heading, [string]$KeySelectorName, $Rows)

    $Lines.Add($Heading)
    $Lines.Add("")
    $Lines.Add("| 分组 | 任务 | 成功 | 跳过 | 失败 | 总渲染时间(s) | 总覆盖时长(s) | 平均每s(ms/s) |")
    $Lines.Add("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |")
    $groups = @($Rows | Group-Object $KeySelectorName | Sort-Object Name)
    foreach ($group in $groups) {
        $ok = @($group.Group | Where-Object { $_.row.status -eq "success" }).Count
        $skip = @($group.Group | Where-Object { $_.row.status -eq "SKIP" }).Count
        $fail = $group.Count - $ok - $skip
        $ms = 0.0
        $sec = 0.0
        foreach ($item in $group.Group) {
            $value = Get-NumberOrNull $item.row.render_ms
            if ($null -ne $value) { $ms += $value }
            $value = Get-NumberOrNull $item.row.covered_s
            if ($null -ne $value -and $item.row.status -eq "success") { $sec += $value }
        }
        $cost = if ($sec -gt 0) { $ms / $sec } else { $null }
        $Lines.Add(("| {0} | {1} | {2} | {3} | {4} | {5:F1} | {6} | {7} |" -f `
            (Escape-MarkdownCell $group.Name), $group.Count, $ok, $skip, $fail, ($ms / 1000.0), `
            (Format-Number $sec 1), (Format-Number $cost 2)))
    }
    $Lines.Add("")
}

# 分组键：格式与模式直接取行字段；先投影到包装对象上便于 Group-Object。
$fmtRows = $allRows | ForEach-Object {
    [pscustomobject]@{ fmt = $_.row.fmt; mode = $_.row.mode; row = $_.row; script = $_.script }
}
Add-GroupSummary $md "### 按输出格式" "fmt" $fmtRows
Add-GroupSummary $md "### 按游戏模式" "mode" $fmtRows

# 渲染效率榜：每s 渲染最高的 10 个成功任务（越低越快，这里列最慢的作为压力参考）。
$md.Add("### 每s 渲染耗时最高的 10 个任务")
$md.Add("")
$md.Add("| 脚本 | 模式 | BID | 格式 | 任务 | 渲染(ms) | 覆盖(s) | 每s(ms) | 峰值(MB) | 备注 |")
$md.Add("| --- | --- | ---: | --- | --- | ---: | ---: | ---: | ---: | --- |")
$slowest = @($allRows | Where-Object { $_.row.status -eq "success" -and (Get-NumberOrNull $_.row.ms_per_s) } |
    Sort-Object { -(Get-NumberOrNull $_.row.ms_per_s) } | Select-Object -First 10)
foreach ($item in $slowest) {
    $md.Add(("| {0} | {1} | {2} | {3} | {4} | {5} | {6} | {7} | {8} | {9} |" -f `
        (Escape-MarkdownCell $item.script), (Escape-MarkdownCell $item.row.mode), (Escape-MarkdownCell $item.row.bid), `
        (Escape-MarkdownCell $item.row.fmt), (Escape-MarkdownCell $item.row.task), `
        (Format-Number (Get-NumberOrNull $item.row.render_ms) 1), (Format-Number (Get-NumberOrNull $item.row.covered_s) 2), `
        (Format-Number (Get-NumberOrNull $item.row.ms_per_s) 2), (Format-Number (Get-NumberOrNull $item.row.peak_mb) 1), `
        (Escape-MarkdownCell $item.row.note)))
}
$md.Add("")

# 总结
$md.Add("## 总结")
$md.Add("")
$md.Add(("- 全部 7 个脚本共 {0} 个任务：成功 {1}、跳过 {2}（PNG 不支持对应 mod）、失败 {3}；全部脚本墙钟 {4:F1}s。" -f `
    $grandTasks, $grandOk, $grandSkip, $grandFail, ($grandWallMs / 1000.0)))
$md.Add(("- 累计渲染时间 {0:F1}s，累计覆盖谱面 {1:F1}s，总体每s渲染 {2}ms/s；单进程峰值内存最大 {3}MB。" -f `
    ($grandMs / 1000.0), $grandCovered, (Format-Number $grandMsPerSec 2), (Format-Number $grandMaxMem 1)))
if ($grandFail -eq 0) {
    $md.Add("- 全部任务渲染成功，PNG 跳过项均为支持矩阵内的正常跳过，渲染结果可用于回归对比。")
} else {
    $md.Add("- 存在失败任务，详见各脚本 ``report.txt`` 的失败详情段；其余任务结果仍可用于回归对比。")
}
$md.Add("- 明细见上文各脚本的单项表格；原始报告见输出目录各 ``batch-*`` 子文件夹下的 ``report.txt``。")
$md.Add("")

$mdPath = Join-Path $outdir "report.md"
$md | Set-Content -LiteralPath $mdPath -Encoding UTF8

if ($UpdateDocs) {
    $docsPath = Join-Path $repoRoot "docs\report.md"
    Copy-Item -LiteralPath $mdPath -Destination $docsPath -Force
    Write-Host "已同步到: $docsPath"
}

# ── 控制台收尾 ──
Write-Host ""
Write-Host ("=" * 130)
Write-Host ("全部完成: 任务 {0}  成功 {1}  跳过 {2}  失败 {3}" -f $grandTasks, $grandOk, $grandSkip, $grandFail)
Write-Host ("总渲染时间 {0:F1}s    总覆盖时长 {1:F1}s    总体每s {2}ms/s    全部脚本墙钟 {3:F1}s" -f `
    ($grandMs / 1000.0), $grandCovered, (Format-Number $grandMsPerSec 2), ($grandWallMs / 1000.0))
Write-Host "总计报告: $txtPath"
Write-Host "Markdown 报告: $mdPath"
Write-Host ("=" * 130)

$anyFailed = $false
foreach ($run in $runs) {
    if ($run.exitCode -ne 0) {
        $anyFailed = $true
        Write-Host ("脚本失败: {0} (exit {1})" -f $run.def.file, $run.exitCode)
    }
}
if ($anyFailed -or $grandFail -gt 0) {
    exit 1
}
