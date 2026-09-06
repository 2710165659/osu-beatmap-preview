param(
    [switch]$Full
)

$ErrorActionPreference = "Stop"
$workspace = Split-Path -Parent $PSScriptRoot
Push-Location $workspace
try {
    cargo test --all-targets
    if ($LASTEXITCODE -ne 0) { throw "默认测试失败" }
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "默认 release 构建失败" }
    cargo test --workspace --all-features --all-targets
    if ($LASTEXITCODE -ne 0) { throw "全 feature 测试失败" }
    cargo build --release --package osu-beatmap-preview-wgpu
    if ($LASTEXITCODE -ne 0) { throw "WGPU exporter 构建失败" }
    cargo test --features wgpu-renderer render::wgpu::offscreen::tests::wgpu回读会移除行对齐填充并保留rgba -- --ignored --exact
    if ($LASTEXITCODE -ne 0) { throw "WGPU readback 验收失败" }
    cargo test --features wgpu-renderer render::wgpu::offscreen::tests::wgpu直接光栅化图元并遵守透明混合与裁剪 -- --ignored --exact
    if ($LASTEXITCODE -ne 0) { throw "WGPU 图元光栅化验收失败" }

    if ($Full) {
        $binary = Join-Path $workspace "target/release/osu-beatmap-preview-wgpu.exe"
        $cases = @(
            @{ Bid = "4897202"; Convert = $null },
            @{ Bid = "1418246"; Convert = $null },
            @{ Bid = "944502"; Convert = $null },
            @{ Bid = "4312004"; Convert = $null },
            @{ Bid = "738063"; Convert = "mania" }
        )
        foreach ($case in $cases) {
            $arguments = @("--bid=$($case.Bid)", "--fmt=mp4", "--duration-time=2", "--fps=30", "--no-cache")
            if ($case.Convert) { $arguments += "--convert=$($case.Convert)" }
            & $binary @arguments
            if ($LASTEXITCODE -ne 0) { throw "BID $($case.Bid) 的完整验收失败" }
        }
    }
} finally {
    Pop-Location
}
