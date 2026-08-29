# osu! Beatmap Preview

[Chinese](../README.md) | [English](README.en.md)

> A fast osu! beatmap preview renderer supporting GIF animations, PNG images, and MP4 videos for Standard, Taiko, Catch, and Mania.

## Features

- **Single executable file**: All resources are embedded into the binary at compile time, with no external dependencies at runtime—ready to use out of the box.  
- **Cross-platform**: Natively supports Windows, Linux, and macOS.  
- **Full-featured**: Supports four game modes, MOD, conversions, and SV functionality.  
- **Three output formats**: GIF animations, static PNG long images, and MP4 videos containing the original chart audio.  
- **High performance**: Video encoding leverages GPU acceleration, and the overall processing pipeline is fast, memory-efficient, and produces small output files. See the [batch rendering report](report.txt) for details.

> If this project is useful to you, please consider giving it a ⭐ Star.

## Usage

```bash
osu-beatmap-preview --bid=<BID> [--convert=mania|ctb|taiko|standard] [--fmt=png|gif|mp4] [--mod=<MOD>]... [--time-points=<SECONDS|preview>]... [--duration-time=<SECONDS>] [--no-log] [--no-cache] [--config=<PATH|JSON|YAML>]
```

### Parameters

| Parameter | Description |
| --- | --- |
| `--bid` | Required. A numeric Beatmap ID. |
| `--convert` | Conversion mode: `mania` / `ctb` / `taiko` / `standard` / `std`. Only available for Standard beatmaps. |
| `--mod` | A single Mod token; repeat the option for combinations, such as `--mod=hd --mod=hr`. |
| `--fmt` | Output format: `gif` / `png` / `mp4`. When omitted, the default format for the mode is used. |
| `--time-points` | A list of gameplay time points. Repeat it for multiple GIF or Standard PNG points; MP4 accepts at most one. Each value is seconds or `preview`. When omitted, points are selected automatically; if GIF or Standard PNG receives only some points, the beatmap `PreviewTime` is added first when capacity remains. MP4 starts at `0` when omitted. |
| `--duration-time` | MP4 only. Output duration in seconds. Defaults to `600`; shorter beatmaps use their complete playable range instead of padding to 600 seconds. |
| `--no-log` | Disables logging. |
| `--no-cache` | Skips download and output caches and forces a fresh render. |
| `--config` | A configuration file path or an inline JSON/YAML object. Nested mappings merge recursively; arrays and scalars replace the whole field. Unspecified fields keep their defaults. |
| `--version` | Prints the version and build time, then exits. |

> MP4 numeric start times use the gameplay timeline: the first playable object in the target mode after conversion is `0:00`. Negative starts are allowed and portions before the audio begins are silent.

### Examples

```bash
# Render with default parameters
osu-beatmap-preview --bid=123456

# Render with conversion parameters
osu-beatmap-preview --bid=123456 --convert=mania

# Combine conversion, Mods, and GIF output
osu-beatmap-preview --bid=123456 --convert=mania --mod=4k --mod=dt1.25 --fmt=gif

# Render with multiple Mods
osu-beatmap-preview --bid=123456 --mod=hd --mod=hr

# Render an MP4 near the beatmap preview time
osu-beatmap-preview --bid=123456 --fmt=mp4 --time-points=preview --duration-time=30

# Specify four GIF render points (repeat the list option)
osu-beatmap-preview --bid=123456 --fmt=gif --time-points=5 --time-points=10 --time-points=15 --time-points=20

# Combine conversion and PNG output; Taiko spacing comes from configuration
osu-beatmap-preview --bid=123456 --convert=taiko --fmt=png --mod=sw

# Force a fresh render and disable logs
osu-beatmap-preview --bid=123456 --no-cache --no-log
```

### Mod Support

| Mode | GIF / MP4 | PNG |
| --- | --- | --- |
| Standard | `EZ` `HR` `HD` `DA` `TC` `DT` `HT` | `EZ` `HR` `HD` `DA` `TC` |
| Taiko | `EZ` `HR` `SW` `CS` `DT` `HT` | `EZ` `HR` `SW` |
| Catch | `EZ` `HR` `DT` `HT` | `EZ` `HR` |
| Mania | `CS` `DT` `HT` `1K`-`10K` `DS` `IN` `HO` | `1K`-`10K` `DS` `IN` `HO` |

### Mod Conflicts

| Combination | Description |
| --- | --- |
| `DT` / `HT` | Mutually exclusive. `DT` defaults to `1.5x` and accepts `1.01-2.00`; `HT` defaults to `0.75x` and accepts `0.50-0.99`. |
| `EZ` / `HR` | Mutually exclusive. |
| `TC` / `HD` | Mutually exclusive. |
| `1K`-`10K` | Mutually exclusive. Takes effect only with `--convert=mania`. |
| `IN` / `HO` | Mutually exclusive. |
| `DA` / `EZ` / `HR` | `DA` cannot be used with `EZ` or `HR`. Standard only. |
| `DA` parameters | Use the format `da<parameter><value>`, such as `dacs5` or `daar9.5`; parameters can also be combined as `dacs5ar9.5`. |

## Output

| Location | Description |
| --- | --- |
| Beatmap cache | `<temp>/osu-beatmap-preview/osu-download-cache/<bid>.osu` |
| OSZ cache | `<temp>/osu-beatmap-preview/osz-download-cache/` (archives use `<set-id>.osz`; extracted audio is isolated under `<set-id>/<filename-hash>.<extension>`) |
| Preferred IP cache | `<temp>/osu-beatmap-preview/osz-download-cache/osu-direct-preferred-ip.json` |
| Output file | `<temp>/osu-beatmap-preview/outputs/<mode>_<bid>[_convert][_mods][_video-start...-duration...].<fmt>`; MP4 adds the time suffix only when a time option is explicitly supplied |
| Configuration directory | Directory containing the binary |
| Log files | `<temp>/osu-beatmap-preview/logs/` — `progress.log` (live progress, `tail -f`) and `render.log` (NDJSON summary) |

These directories are defined by the top-level `paths` entries in `assets/default_config.yml`; the default `CONFIG_DIR: "./"` points to the directory containing the binary. The program tries to read `config.yml` beside the binary, then applies the `--config` overlay. A missing file is ignored; an invalid existing file fails startup.

Configuration layers are merged as `embedded defaults < CONFIG_DIR/config.yml < --config`. `--config` accepts a file path or an inline JSON/YAML object, for example:

Fields must exist in the embedded configuration. Unknown fields, a non-object top level, and values that cannot be converted to the expected type fail startup. Numeric and boolean strings are accepted when safely convertible.

Whole-request timeouts for PNG, GIF, and MP4 are controlled by `timeouts.render.PNG_TIMEOUT`, `GIF_TIMEOUT`, and `MP4_TIMEOUT`. Values are positive integer seconds and default to `300`. Timing starts at the request entry point and covers download, parsing, conversion, cache lookup, rendering, audio processing, encoding, and final output. When `--fmt` is omitted and the beatmap mode is not known before download, the preliminary phase uses the larger PNG/GIF timeout; once the mode is known, the actual format deadline is recalculated from the original request start.

```yaml
timeouts:
  render:
    PNG_TIMEOUT: 300
    GIF_TIMEOUT: 300
    MP4_TIMEOUT: 300
```

```bash
osu-beatmap-preview --bid=123456 --config='{"layout":{"standard":{"gif":{"ROW_COUNT":1}}}}'
osu-beatmap-preview --bid=123456 --config='{layout: {standard: {gif: {ROW_COUNT: 1}}}}'
osu-beatmap-preview --bid=123456 --config=C:/path/to/config.yml
```

### Images (PNG)

Use `--fmt=png` to output a static PNG image.

- Standard outputs a configurable GIF grid; row and column counts come from `assets/default_config.yml`.
- Taiko outputs a scroll-layout chart image; spacing comes from `SPACING_PER_BPM` in configuration (`0` means automatic).
- Catch outputs a chart image arranged along the beatmap.
- Mania outputs a lane-based chart image and automatically splits long beatmaps into multiple columns.

### GIF Animations

Use `--fmt=gif` to output a GIF animation. Each mode has independent grid, duration, and `SHOW_TIME_LABEL` settings in `assets/default_config.yml`. A `1x1` grid with labels enabled provides a single-panel labeled preview. Segment selection is deterministic and starts from `PreviewTime` when available.

### MP4 Videos

Use `--fmt=mp4` to output a video with beatmap audio for all four modes. The default request is 600 seconds from gameplay time `0`; shorter beatmaps use their complete playable range. Use one `--time-points` value and `--duration-time` to select another interval; intervals that run past the chart tail are shifted backward as a unit. The default output name omits the time suffix; the suffix is kept when either option is explicitly supplied. `--time-points=preview` uses the beatmap `PreviewTime`. GIF and Standard PNG accept repeated `--time-points` values and prioritize adding `PreviewTime` when capacity remains. The old `5+10+15` joined format is invalid.

### Command-Line Output

The program prints a JSON object to stdout. Its schema is as follows:

```json
{
  "status": "success",
  "msg": "preview generated successfully for bid 738063",
  "preview-img": "/path/to/output.png",
  "beatmap-info": {
    "meta-data": { "title": "...", "artist": "...", ... },
    "difficulty": { ... }
  }
}
```

> `preview-img` is an absolute path. Its extension follows `--fmt`: `.gif`, `.png`, or `.mp4`.

## Preview

![Overview](total.png)

## Other Notes

### Parameter Restrictions

- `--mod` must be repeated for each Mod token; `+`-joined Mod strings are invalid.
- `--time-points` is valid for GIF, Standard PNG, and MP4; `--duration-time` is valid only for MP4.

### Caches and Logs

Logging is enabled by default and always uses the configured `LOG_DIR`; use `--no-log` to disable it. A logging failure only falls back to a message on stderr and does not affect rendering or stdout results.

Cache files are not deleted automatically and can be removed manually from the temporary directory when they take up too much space. Output files are written atomically: a temporary file is completed before it replaces the final file, so an interrupted render does not leave a corrupted file that can be treated as a valid cache.

## Build

### Requirements

Rust 1.73+ and a C++ compiler are required. Install Rust from <https://rustup.rs>.

### Build Command

```bash
cargo build --release
# output: target/release/osu-beatmap-preview(.exe)
```

## License

[MIT](../LICENSE). The embedded AAC encoder uses Fraunhofer FDK-AAC; its license does not grant patent rights. See [Third-party notices](THIRD_PARTY_NOTICES.md).
