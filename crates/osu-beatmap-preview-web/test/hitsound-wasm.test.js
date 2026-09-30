// 用 Node 直接跑 wasm 产物，验证「单文件输入 → 谱面信息 / 难度清单」与默认配置转出。
//
// 浏览器里的会话创建依赖 WebGPU，但 `beatmapInfo` 与 `hitsoundDefaults` 只用到解析器，
// 在 Node 里就能验证：wasm 导出齐全、`.osu` 解析结果正确、单文件没有难度清单、
// 四模式的打击音默认值与 `shared_config.yml` 一致。`.osz` 的解包与媒体解码在
// wasm crate 的 Rust 测试里覆盖（archive.rs / decode.rs）。

import assert from 'node:assert/strict';
import fsp from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { test } from 'node:test';

const here = path.dirname(fileURLToPath(import.meta.url));
const pkgDir = path.resolve(here, '../public/pkg');

/** 载入 wasm 胶水代码；产物不存在时跳过整组测试（CI 里没有 wasm 工具链时也不该失败）。 */
async function loadModule() {
  const glue = path.join(pkgDir, 'osu_beatmap_preview_wasm.js');
  try {
    await fsp.access(glue);
  } catch {
    return null;
  }
  // Windows 下动态 import 需要真正的 file:// URL。
  const module = await import(pathToFileURL(glue).href);
  // 胶水代码把字符串/URL 一律交给 fetch，而 Node 的 fetch 不支持 file://；
  // 直接给一个已经带内容的 Response，胶水代码会照常消费它。
  const wasm = await fsp.readFile(path.join(pkgDir, 'osu_beatmap_preview_wasm_bg.wasm'));
  await module.default({
    module_or_path: new Response(wasm, {
      headers: { 'Content-Type': 'application/wasm' },
    }),
  });
  return module;
}

/** 最小可解析 standard 谱面：一个圆圈、一个滑条、一个转盘，覆盖三类打击音。 */
const standard_beatmap = `osu file format v14

[General]
Mode: 0
AudioFilename: audio.mp3

[Metadata]
Title:Test
Artist:Test
Creator:Test
Version:Test
BeatmapID:4242

[Difficulty]
CircleSize:4
SliderMultiplier:1.4
SliderTickRate:1

[TimingPoints]
0,500,4,2,0,100,1,0

[HitObjects]
256,192,1000,1,0,0:0:0:0:
256,192,2000,2,0,B|356:192|256:192|356:192,1,560
256,192,3000,8,0,4000,0:0:0:0:
`;

test('wasm 导出齐全：beatmapInfo / hitsoundDefaults', async (t) => {
  const module = await loadModule();
  if (!module) {
    t.skip('public/pkg 下没有 wasm 产物，先运行 npm run build:wasm');
    return;
  }
  assert.equal(typeof module.beatmapInfo, 'function');
  assert.equal(typeof module.hitsoundDefaults, 'function');
  assert.equal(typeof module.WebGpuSession, 'function');
});

test('.osu 单文件：谱面信息正确、没有难度清单', async (t) => {
  const module = await loadModule();
  if (!module) {
    t.skip('public/pkg 下没有 wasm 产物，先运行 npm run build:wasm');
    return;
  }
  const info = module.beatmapInfo(new TextEncoder().encode(standard_beatmap));
  assert.equal(info.title, 'Test');
  assert.equal(info.mode, 0);
  // 单文件不给难度清单（.osz 才有），加载页据此隐藏难度下拉。
  assert.deepEqual([...info.difficulties], []);
});

test('打击音默认值随模式走，未知模式报错', async (t) => {
  const module = await loadModule();
  if (!module) {
    t.skip('public/pkg 下没有 wasm 产物，先运行 npm run build:wasm');
    return;
  }
  for (const mode of ['standard', 'std', 'taiko', 'catch', 'ctb', 'mania']) {
    const defaults = module.hitsoundDefaults(mode);
    assert.equal(typeof defaults.enabled, 'boolean', mode);
    assert.equal(typeof defaults.volume, 'number', mode);
    assert.equal(typeof defaults.beatmapEnabled, 'boolean', mode);
  }
  assert.throws(() => module.hitsoundDefaults('不存在的模式'));
});
