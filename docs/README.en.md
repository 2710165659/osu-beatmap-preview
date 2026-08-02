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
osu-beatmap-preview --bid=<BID> [--convert=mania|ctb|taiko|standard|std] [--mods=<MODS>] [--fmt=png|gif|mp4] [--time=<T1+T2+...>] [--gif-clip] [--gif-clip-label] [--preview-30s] [--gap=<BPM>] [--log-dir=<DIR>] [--no-log] [--no-cache]
```

Parameter aliases: `--mod` = `--mods`, `--format` = `--fmt`, `--times` = `--time`.

### Parameters

| Parameter | Description |
| --- | --- |
| `--bid` | Required. A numeric Beatmap ID. |
| `--convert` | Conversion mode: `mania` / `ctb` / `taiko` / `standard` / `std`. Only available for Standard beatmaps. |
| `--mods` | Mod combination joined with `+`, such as `hd+hr`; speed-changing Mods may include a value, such as `dt1.25`. |
| `--fmt` | Output format: `gif` / `png` / `mp4`. When omitted, the default format for the mode is used. |
| `--time` | Time point or range on the gameplay skin timeline, in seconds. The first playable object is `0:00`; negative values are supported. |
| `--gif-clip` | GIF only. Outputs a single-screen continuous GIF without time labels. |
| `--gif-clip-label` | GIF only. Like `--gif-clip`, but shows time labels. |
| `--preview-30s` | MP4 only. Renders about 30 seconds of actual playback near `PreviewTime`. |
| `--gap` | Taiko PNG only. Sets the layout spacing BPM. |
| `--log-dir` | Sets the log directory. |
| `--no-log` | Disables logging. |
| `--no-cache` | Skips download and output caches and forces a fresh render. |
| `--version` | Prints the version and build time, then exits. |

> `--time` uses the same timeline as osu!'s in-game song-progress skin components: the first playable object in the target mode after conversion is `0:00`, rather than the editor's absolute track time. Regular GIF and Standard PNG accept 1-4 time points; MP4, `--gif-clip`, and `--gif-clip-label` require a two-point range. Negative values select time before the first object; use the equals form, for example `--time=-2+10`. MP4 portions before the audio begins are silent.

### Examples

```bash
# Render with default parameters
osu-beatmap-preview --bid=123456

# Render with conversion parameters
osu-beatmap-preview --bid=123456 --convert=mania

# Combine conversion, Mods, and GIF output
osu-beatmap-preview --bid=123456 --convert=mania --mods=4k+dt1.25 --fmt=gif

# Render with multiple Mods
osu-beatmap-preview --bid=123456 --mods=hd+hr

# Specify multiple GIF preview time points
osu-beatmap-preview --bid=123456 --fmt=gif --time=10+25+60

# Render a continuous GIF starting two seconds before the first object
osu-beatmap-preview --bid=123456 --fmt=gif --gif-clip-label --time=-2+10

# Specify a range for a continuous GIF without time labels
osu-beatmap-preview --bid=123456 --fmt=gif --gif-clip --time=30+42

# Combine an MP4 time range with Mods
osu-beatmap-preview --bid=123456 --fmt=mp4 --time=30+60 --mods=hd+dt1.25

# Render about 30 seconds of MP4 near the beatmap preview time
osu-beatmap-preview --bid=123456 --fmt=mp4 --preview-30s

# Combine conversion, PNG output, and layout spacing
osu-beatmap-preview --bid=123456 --convert=taiko --fmt=png --gap=180 --mods=sw

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
| Output file | `<temp>/osu-beatmap-preview/outputs/<mode>_<bid>[_convert][_mods][_gifclip][_t<time-or-range>][_preview30s][_bpm<BPM>].<fmt>` |
| Log files | `<temp>/osu-beatmap-preview/logs/` - `progress.log` (live progress, `tail -f`) and `render.log` (NDJSON summary) |

### Images (PNG)

Use `--fmt=png` to output a static PNG image.

- Standard outputs a `5x8` preview grid with 5 rows and 8 consecutive frames per row; `--time` accepts up to 4 time points, with each point used as the start of a preview row.
- Taiko outputs a scroll-layout chart image; use `--gap=<BPM>` to adjust the layout spacing.
- Catch outputs a chart image arranged along the beatmap.
- Mania outputs a lane-based chart image and automatically splits long beatmaps into multiple columns.

### GIF Animations

Use `--fmt=gif` to output a GIF animation. By default, it outputs multiple preview segments; use `--time=t1+t2+...` to specify points on the gameplay skin timeline. `--gif-clip` outputs a single-screen continuous GIF without time labels; `--gif-clip-label` uses the same format with labels. Without an explicit range, segment selection still uses the absolute `.osu` `PreviewTime`, while labels are converted to first-object-relative time. If the tail is too short, the range is shifted backward.

### MP4 Videos

Use `--fmt=mp4` to output a video with beatmap audio for all four modes. The default covers the full beatmap; use `--time=t1+t2` to specify a range on the gameplay skin timeline. The top-right label shows the current skin time on the left and the full playable duration from the first object through the last object's end on the right; the total is independent of the export range and padding. Negative time and the default full-chart padding are shown with negative labels, and portions before the audio file begins are silent. `--preview-30s` renders about 30 seconds near the absolute `.osu` `PreviewTime` and cannot be used with `--time`. If the preview is near the end, the range is shifted backward.

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
  },
  "build-info": {
    "version": "1.0.3",
    "build_time": "2026-06-22T16:01:06.623636800Z"
  },
  "log": {
    "progress": "C:/Users/.../AppData/Local/Temp/osu-beatmap-preview/logs/progress.log",
    "render": "C:/Users/.../AppData/Local/Temp/osu-beatmap-preview/logs/render.log"
  }
}
```

> `preview-img` is an absolute path. Its extension follows `--fmt`: `.gif`, `.png`, or `.mp4`.
> `log` is optional and is present only when logging is enabled; it does not affect existing parsers.

## Preview

![Overview](total.png)

## Other Notes

### Parameter Restrictions

- `--gif-clip` and `--gif-clip-label` are mutually exclusive and can only be used for GIF output.
- `--preview-30s` can only be used for MP4 output and cannot be combined with `--time`.
- When used with MP4, `--time` must contain exactly two time points in the format `--time=t1+t2`.
- When used with `--gif-clip` or `--gif-clip-label`, `--time` must contain exactly two time points in the format `--time=t1+t2`.
- `--gap` only takes effect for Taiko PNG output.

### Caches and Logs

Logging is enabled by default. Use `--log-dir=<DIR>` to set the directory or `--no-log` to disable logging; the `OSU_PREVIEW_LOG_DIR` environment variable can also override the directory. A logging failure only falls back to a message on stderr and does not affect rendering or stdout results.

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
