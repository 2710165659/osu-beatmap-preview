# osu! Beatmap Preview

[Chinese](../README.md) | [English](README.en.md)

A standalone osu! beatmap preview tool for osu!standard, osu!taiko, osu!catch, and osu!mania. It exports beatmaps as PNG images, GIF animations, or MP4 videos with the original audio, and can also play a beatmap in real time in the browser with WebGPU.

![Rendering results for all four modes](total.png)

## Highlights

- **Mods**: `EZ` `HR` `HD` `DA` `TC` `SW` `CS` `DS` `IN` `HO` and `1K`-`10K` key counts combine freely, `DT`/`HT` accept custom speed multipliers (`1.01`-`2.00x` / `0.50`-`0.99x`), and `DA` tunes `cs`/`ar`/`od`/`hp` individually. Duplicate, conflicting (such as `EZ` with `HR`), or mode-incompatible Mods are rejected instead of being silently ignored.
- **Conversion**: Standard beatmaps convert to Taiko, Catch, or Mania, where `1K`-`10K`, `DS`, `IN`, and `HO` affect the result. Converting to the source mode is treated as no conversion.
- **Four modes**: osu!standard, osu!taiko, osu!catch, and osu!mania each have their own layout, skin, and colors, and each can export PNG overviews, segmented GIF previews, and H.264 MP4 videos with the original audio.
- **Fast and lightweight**: Frame rendering runs in configurable parallel chunks whose batch size is bounded by the per-frame byte count, keeping temporary memory peaks low on large canvases. GIF encoding reuses its frame buffer, Standard MP4 precomputes the visible objects per frame, and slider ticks and drum-roll ticks use procedural sprite caches. See the [batch rendering report](report.md) for measured numbers.
- **Standalone CLI with no external dependencies**: A single executable embeds skins, fonts, and the H.264 and AAC encoders, so no FFmpeg, extra shared libraries, or resource files are needed at runtime. Windows tries NVENC and AMF automatically and falls back to the built-in CPU encoder (`OSU_PREVIEW_NO_GPU=1` forces CPU). Download and output caches are reused locally, with a separate output directory per effective configuration.

See the [CLI guide](../crates/osu-beatmap-preview-cli/README.md) for the Mod syntax, every option, and the configuration reference.

## Documentation

The CLI is the main way to use the project, and its full documentation lives in the CLI README. This file only introduces the project and its architecture.

| Document | Content |
| --- | --- |
| [**CLI guide**](../crates/osu-beatmap-preview-cli/README.md) | Download, quick start, every command-line option, output formats, Mods, configuration, cache and logs |
| [WASM guide](../crates/osu-beatmap-preview-wasm/README.md) | Building the WASM bundle, JavaScript API, host responsibilities and limitations |
| [Architecture](architecture.md) | Crate split, API boundaries, scene and WGPU backend, non-goals |
| [Batch rendering report](report.md) | Performance and resource usage data |
| [Third-party notices](THIRD_PARTY_NOTICES.md) | Licenses and source offers for embedded codecs |

The CLI and WASM guides are currently written in Chinese.

## Release artifacts

Two artifacts are published independently:

| Artifact | Form | Notes |
| --- | --- | --- |
| CLI | Single-file executable per platform | `osu-beatmap-preview-<platform>-<arch>-cli`, exports PNG/GIF/MP4 |
| WASM | `osu-beatmap-preview-wasm-web.zip` | Real-time WebGPU rendering driven by a host web page |

See [Releases](https://github.com/2710165659/osu-beatmap-preview/releases). Neither artifact needs FFmpeg or extra resource files.

## Architecture

The code is split into a cross-platform core, a GPU rendering layer, and platform adapters. CPU export and real-time GPU rendering are two independent paths:

```text
CLI  -> cli layer -> arguments, config, download, cache, logs, media encoding, file output
                  `-> core: parsing, Mods, conversion, timeline, per-mode CPU frames and static scenes -> PNG/GIF/MP4

Web  -> host fetches bytes -> wasm -> core RealtimeSession -> renderer WebGPU canvas
```

| Crate | Responsibility |
| --- | --- |
| `osu-beatmap-preview-core` | Beatmap model, parsing, Mods, conversion, timeline, and the per-mode CPU frame and static scene rendering that produces a `FrameScene` free of window, network, or audio device dependencies |
| `osu-beatmap-preview-renderer` | Platform-independent WGPU drawing; the host provides the device, queue, surface, and target view |
| `osu-beatmap-preview-cli` | Command-line arguments, config, download, cache, logs, timeline and layout assembly, media encoding, and file output (default build target, release artifact) |
| `osu-beatmap-preview-wasm` | Connects the core session and renderer to a browser WebGPU canvas (release artifact) |
| `osu-beatmap-preview-player` | Local web debug player, not part of the release |
| `osu-beatmap-preview-gui`, `osu-beatmap-preview-mobile` | Desktop and mobile skeletons for surface, input, playback lifecycle, and audio clock adapters |

The CLI always uses the CPU export path, does not depend on the renderer crate, and exposes no WGPU option. The real-time API reports an explicit error when the GPU is unavailable instead of silently falling back to CPU rendering. See the [architecture document](architecture.md) for crate boundaries, configuration sources, and non-goals.

## Build from source

A stable Rust toolchain and a working C/C++ build environment are required; see <https://rustup.rs>.

```bash
git clone https://github.com/2710165659/osu-beatmap-preview.git
cd osu-beatmap-preview
cargo build --release
```

The only default workspace member is the CLI, so the output is `target/release/osu-beatmap-preview-cli` (`osu-beatmap-preview-cli.exe` on Windows). For the WASM bundle, see the [WASM guide](../crates/osu-beatmap-preview-wasm/README.md).

Run the tests:

```bash
cargo test --workspace --all-features --all-targets
```

## License

This project is licensed under the [MIT License](../LICENSE). The embedded AAC encoder uses Fraunhofer FDK-AAC, whose license grants no patent rights; see the [third-party notices](THIRD_PARTY_NOTICES.md).
