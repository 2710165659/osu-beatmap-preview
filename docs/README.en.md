<div align="center">

# osu! Beatmap Preview

[![Release](https://img.shields.io/github/v/release/2710165659/osu-beatmap-preview?color=FF6600)](https://github.com/2710165659/osu-beatmap-preview/releases)
[![License](https://img.shields.io/badge/License-MIT-2EA44F)](../LICENSE)
[![Rust](https://img.shields.io/badge/Rust-stable-000000)](https://www.rust-lang.org/)

[中文](../README.md) | [English](README.en.md)

</div>

A standalone osu! beatmap and replay preview tool. Export beatmaps from all four modes as PNG, GIF, or MP4, or play them in the browser. Replay preview is also available.

![Rendering results for all four modes](total.png)

## Features

- **Four beatmap modes**: osu!standard, osu!taiko, osu!catch, and osu!mania, with static overviews, segmented animations, and videos with music.
- **Replay preview**: load local OSR files, online scores, or player scores to display cursor movement, input, judgements, and the score HUD. Export PNG/GIF/MP4 with the CLI or play interactively on the web.
- **Mods and conversion**: adjust difficulty, visibility, speed, and mode-specific settings. Convert Standard maps to Taiko, Catch, or Mania; see the [CLI Mod reference](../crates/osu-beatmap-preview-cli/README.md#mod-支持) for supported combinations.
- **Media**: MP4 and web playback support backgrounds, background video, storyboards, and hitsounds, preferring samples supplied by the beatmap. Video and storyboards are disabled by default in the CLI and enabled by default in the web interface.
- **Local and online files**: download by beatmap ID, load local OSU/OSZ files, or pair an external beatmap with its media archive. Replays validate the original beatmap bytes against the recorded MD5.
- **Standalone CLI**: skins, fonts, H.264, and AAC are embedded. No FFmpeg or extra runtime resources are required. Windows tries NVENC/AMF and falls back to CPU encoding.

## Quick start

Download the CLI or web package from [Releases](https://github.com/2710165659/osu-beatmap-preview/releases). Each works independently.

### CLI export

Binaries are available for Windows x64, Linux x64, macOS Intel, and Apple Silicon. These examples assume the binary is named `osu-beatmap-preview-cli` and is on `PATH`. Linux/macOS users must first grant execute permission.

```bash
osu-beatmap-preview-cli --bid=738063
osu-beatmap-preview-cli --osz-file=map.osz --bid=738063 --fmt=mp4
osu-beatmap-preview-cli --type=osr --osr-file=play.osr --osz-file=map.osz
```

Beatmap defaults are GIF for Standard and PNG for other modes. Replay export defaults to MP4 and requires a matching complete archive; replay PNG/GIF needs matching OSU and OSR files. Successful requests return JSON on stdout, with the absolute output path in `preview-img`.

See the [CLI guide](../crates/osu-beatmap-preview-cli/README.md) for parameters, segment selection, Mods, and configuration.

### Web playback

The release web package requires Node.js 18+ and a WebGPU-capable browser. Unzip it and run:

```bash
node backend/server.js
```

Open [http://127.0.0.1:8787](http://127.0.0.1:8787) and choose beatmap or replay preview. Playback supports seeking, speed, volume, resolution, and frame-rate controls. Online share links use `/bids/<beatmapID>` and `/scores/<scoreID>`.

Online replay downloads and MD5-based beatmap lookups require official API credentials configured by the server operator. A local replay paired with a matching complete OSZ works offline. See the [web guide](../crates/osu-beatmap-preview-web/README.md) for credentials and HTTPS deployment.

## Documentation

| Document | Contents |
| --- | --- |
| [CLI guide](../crates/osu-beatmap-preview-cli/README.md) | Downloads, beatmap/replay export, parameters, Mods, configuration, caches, and logs |
| [Web guide](../crates/osu-beatmap-preview-web/README.md) | Input sources, share links, server options, endpoints, and deployment |
| [WASM guide](../crates/osu-beatmap-preview-wasm/README.md) | Building, JavaScript APIs, audio integration, and host responsibilities |
| [Batch rendering report](report.md) | Recorded performance and resource usage |
| [Changelog](../CHANGELOG.md) | Main changes by version |
| [Third-party notices](THIRD_PARTY_NOTICES.md) | Codec, font, sound, and frontend licenses |

The detailed guides are written in Chinese.

## Build from source

The CLI requires stable Rust and a working C/C++ toolchain:

```bash
git clone https://github.com/2710165659/osu-beatmap-preview.git
cd osu-beatmap-preview
cargo build --release
cargo test --workspace --all-features --all-targets
```

The default workspace member is the CLI. Its output is `target/release/osu-beatmap-preview-cli` (`.exe` on Windows).

Frontend builds require Node.js 20.19+. For WASM, install the `wasm32-unknown-unknown` target and a `wasm-bindgen-cli` matching `Cargo.lock`, as described in the [WASM guide](../crates/osu-beatmap-preview-wasm/README.md#构建). Then run:

```bash
cd crates/osu-beatmap-preview-web
npm install
npm run build:wasm
npm run build
npm test
npm start
```

On Windows, `run_web.ps1` in that directory installs dependencies, builds, and starts the server. Docker deployment is covered in the [web guide](../crates/osu-beatmap-preview-web/README.md#docker-部署).

## License

Project code is licensed under [MIT](../LICENSE). Embedded codecs, Torus fonts, and replay sounds retain their own licenses; see the [third-party notices](THIRD_PARTY_NOTICES.md).
