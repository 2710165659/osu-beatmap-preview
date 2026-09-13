// 跨实现契约测试：`.osu` 的下载字段与压缩包条目规则在 core（Rust）、Node 后端（JS）
// 与 wasm 之间必须一致。
//
// 后端为了「无运行时依赖」自己实现了一份最小 `.osu` 解析（beatmap-meta.js）与 ZIP 读取
// （zip.js），core 也有一份媒体策略（domain/media.rs）。这里把两边的输出钉在一起：
// 任一实现改规则而另一边没跟上，测试就会失败。

import assert from 'node:assert/strict';
import fs from 'node:fs';
import fsp from 'node:fs/promises';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { parseBeatmapText } from '../backend/beatmap-meta.js';
import { extractEntry, findEntry, normalizeArchivePath, readZipIndex } from '../backend/zip.js';

const here = path.dirname(fileURLToPath(import.meta.url));
const fixture = fs.readFileSync(path.join(here, 'fixtures/sample.osz'));
const pkgDir = path.resolve(here, '../public/pkg');

/**
 * 一份**完整**的最小谱面：core 的 `parse_beatmap_bytes` 要求
 * `[Difficulty]` / `[TimingPoints]` / `[HitObjects]` 都存在，而随 OSZ 分发的 fixture
 * 里的 `map.osu` 只有下载需要的字段（后端只取三个字段，所以能解析）。
 * 两个实现必须在「同一份文件」上给出一致的结果，因此这里用完整文档做对照。
 */
const CONTRACT_BEATMAP = `osu file format v14

[General]
AudioFilename: audio/song.mp3
AudioLeadIn: 1500
Mode: 0

[Metadata]
Title:Contract
BeatmapSetID:4242

[Difficulty]
CircleSize:4
SliderMultiplier:1.4
SliderTickRate:1

[TimingPoints]
0,500,4,2,0,100,1,0

[Events]
0,0,"backgrounds\\bg.jpg",0,0

[HitObjects]
256,192,1000,1,0,0:0:0:0:
`;

/** 上面那份谱面在两个实现里必须得到的字段（背景名会被归一化成 `/`）。 */
const CONTRACT_EXPECTED = {
  audioFilename: 'audio/song.mp3',
  backgroundFilename: 'backgrounds/bg.jpg',
  beatmapSetId: 4242,
  audioLeadInMs: 1500,
};

/** 从 fixture 里取出 `.osu` 字节。 */
function beatmapBytes() {
  const entries = readZipIndex(fixture);
  const entry = findEntry(entries, 'map.osu');
  assert.ok(entry, 'fixture 里必须有 map.osu');
  return extractEntry(fixture, entry);
}

/** 载入 wasm 胶水代码；产物不存在时跳过整组测试（CI 里没有 wasm 工具链时也不该失败）。 */
async function loadModule() {
  const glue = path.join(pkgDir, 'osu_beatmap_preview_wasm.js');
  try {
    await fsp.access(glue);
  } catch {
    return null;
  }
  const module = await import(pathToFileURL(glue).href);
  // Windows 下动态 import 需要真正的 file:// URL；Node 的 fetch 不支持 file://，
  // 因此直接给一个已经带内容的 Response。
  const wasm = await fsp.readFile(path.join(pkgDir, 'osu_beatmap_preview_wasm_bg.wasm'));
  await module.default({
    module_or_path: new Response(wasm, {
      headers: { 'Content-Type': 'application/wasm' },
    }),
  });
  return module;
}

test('后端能从随 OSZ 分发的精简 .osu 里取出下载字段', () => {
  // fixture 里的 map.osu 只写了 General / Metadata / Events：后端只需要这三个字段，
  // core 的完整解析则会因为缺少 [Difficulty] / [TimingPoints] / [HitObjects] 而拒绝它。
  const beatmap = parseBeatmapText(beatmapBytes().toString('utf8'));
  assert.equal(beatmap.audioFilename, 'audio.mp3');
  assert.equal(beatmap.backgroundFilename, 'bg.jpg');
  assert.equal(beatmap.beatmapSetId, 4242);
});

test('同一份完整 .osu 在后端与 wasm 里得到相同字段', async (t) => {
  const module = await loadModule();
  if (!module) {
    t.skip('public/pkg 下没有 wasm 产物，先运行 npm run build:wasm');
    return;
  }
  const backend = parseBeatmapText(CONTRACT_BEATMAP);
  const info = module.beatmapInfo(new TextEncoder().encode(CONTRACT_BEATMAP));

  // 先钉住两边的具体值，避免两份实现一起漂移。
  assert.equal(info.audioFilename, CONTRACT_EXPECTED.audioFilename);
  assert.equal(info.backgroundFilename, CONTRACT_EXPECTED.backgroundFilename);
  assert.equal(info.beatmapSetId, CONTRACT_EXPECTED.beatmapSetId);
  assert.equal(info.audioLeadInMs, CONTRACT_EXPECTED.audioLeadInMs);

  // 再钉住两边一致（字段名不同：后端是 camelCase，wasm 的 beatmapInfo 也是 camelCase）。
  assert.equal(backend.audioFilename, info.audioFilename);
  assert.equal(backend.backgroundFilename, info.backgroundFilename);
  assert.equal(backend.beatmapSetId, info.beatmapSetId);
});

test('压缩包路径归一化与 core 的用例表一致', () => {
  // 与 `crates/osu-beatmap-preview-core/src/domain/media.rs` 的单测同一张表。
  assert.equal(normalizeArchivePath('audio\\song.mp3'), 'audio/song.mp3');
  assert.equal(normalizeArchivePath('a/./b//c'), 'a/b/c');
  assert.equal(normalizeArchivePath(' bg.jpg '), 'bg.jpg');
  assert.equal(normalizeArchivePath('./hitnormal.ogg'), 'hitnormal.ogg');
  assert.equal(normalizeArchivePath('../song.mp3'), null);
  assert.equal(normalizeArchivePath('a/../../b'), null);
  assert.equal(normalizeArchivePath('C:/song.mp3'), null);
  assert.equal(normalizeArchivePath('/song.mp3'), null);
  assert.equal(normalizeArchivePath('a/b:c'), null);
  assert.equal(normalizeArchivePath('   '), null);
  assert.equal(normalizeArchivePath('.'), null);
});
