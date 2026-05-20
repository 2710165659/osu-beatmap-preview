---
name: osu-beatmap-preview
description: Generate osu! beatmap preview images and structured JSON results from beatmap ids. Use this skill when a task involves osu! beatmap preview rendering, inspecting standard/taiko/catch/mania charts, converting osu!standard beatmaps to taiko/catch/mania previews, applying preview mods, or running `python scripts/run.py` to return preview paths plus metadata and difficulty fields.
---

# osu! Beatmap Preview

Run commands from the skill root, or use absolute paths when working from another directory.

Use the CLI entrypoint:

```bash
python scripts/run.py --bid=<BID> [选项]
```

Prefer these flags in new commands:

- `--mods` for mods. `--mod` is also accepted.
- `--time` for GIF time points. `--times` is also accepted.

Use these common options:

- `--convert=mania|ctb|taiko` to convert an osu!standard beatmap before rendering.
- `--fmt=gif|png` to force the output format.
- `--mods=...` to apply preview mods.
- `--time=10+25+60` to choose GIF start times in seconds.

Follow the output contract:

- Read the JSON from stdout.
- Use `preview-img` from the JSON instead of guessing the output path.
- Treat `status=success` as a successful render.
- Treat `status=error` as a handled failure and return the provided `msg`.
- Keep `beatmap-info.meta-data` and `beatmap-info.difficulty` unchanged if forwarding the result.

Remember these behavior rules:

- `standard` defaults to GIF.
- `taiko`, `catch`, and `mania` default to PNG.
- `--convert` only works for osu!standard source beatmaps.
- `--time` is only valid for GIF output.
- The command downloads missing `.osu` files on demand, so network access may be required on first run.

Use these mod sets:

- `standard` GIF/PNG: `EZ` `HR` `HD` `DA` `DT` `HT`
- `taiko` GIF: `EZ` `HR` `SW` `CS` `DT` `HT`
- `taiko` PNG: `EZ` `HR` `SW`
- `catch` GIF: `EZ` `HR` `DT` `HT`
- `catch` PNG: `EZ` `HR`
- `mania` GIF: `1K`-`10K` `DS` `CS` `IN` `HO` `DT` `HT`
- `mania` PNG: `1K`-`10K` `DS` `IN` `HO`

Remember these mod limits:

- Join multiple mods with `+`, for example `hd+hr` or `4k+in+dt1.2`.
- `DT` defaults to `1.5x` and accepts `1.01-2.00`, for example `dt1.25`.
- `HT` defaults to `0.75x` and accepts `0.50-0.99`, for example `ht0.8`.
- `DA` is `standard`-only and uses forms like `dacs5ar9.5`, `daod8hp3`.
- `DA` conflicts with `EZ` and `HR`.
- `DT` conflicts with `HT`.
- `EZ` conflicts with `HR`.
- `1K`-`10K` are mutually exclusive.
- `IN` conflicts with `HO`.
- `1K`-`10K` and `DS` only have real effect when converting `standard` to `mania`.
- `taiko` `CS` is GIF-only.
- `mania` `CS` is GIF-only.

Use simple commands like these:

```bash
python scripts/run.py --bid=5199917
python scripts/run.py --bid=123456 --convert=mania --fmt=gif --mods=4k+in --time=10+30
python scripts/run.py --bid=123456 --mods=dt1.25
```
