// 用 Node 直接跑 wasm 产物，验证「谱面 → 需要的样本名 → 内嵌资源字节」这条链路。
//
// 浏览器里混音依赖 WebGPU（会话创建），但样本名解析与资源取字节只用到解析器与内嵌
// 资源表，因此在 Node 里就能完整验证：能确认 wasm 导出存在、四模式的音效集合符合
// 预期，以及每个需要的样本都真的内嵌在 wasm 里（这正是「Web 端音效全都没有」的根因）。

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

/** 最小 standard 谱面：一个圆圈、一个滑条、一个转盘，覆盖三类打击音。 */
const standard_beatmap = `osu file format v14

[General]
Mode: 0
AudioFilename: audio.mp3

[Metadata]
Title:Test
Artist:Test
Creator:Test
Version:Test

[Difficulty]
CircleSize:4
SliderMultiplier:1.4
SliderTickRate:1

[TimingPoints]
0,500,4,2,0,100,1,0

[HitObjects]
256,192,1000,1,0,0:0:0:0:
256,192,2000,2,0,B|356:192|256:192|356:192,1,560
256,192,4000,8,0,6000
`;

/** 最小 taiko 谱面：红音符与蓝音符各一个，音量 90 落在 drum 档。 */
const taiko_beatmap = `osu file format v14

[General]
Mode: 1
AudioFilename: audio.mp3

[Metadata]
Title:Test
Artist:Test
Creator:Test
Version:Test

[Difficulty]
CircleSize:5

[TimingPoints]
0,500,4,3,0,90,1,0

[HitObjects]
256,192,1000,1,0
256,192,1500,1,8
`;

/** 低音量的 taiko 谱面：音量 50 落在 soft 档。 */
const taiko_soft_beatmap = taiko_beatmap.replace(',4,3,0,90,1,0', ',4,3,0,50,1,0');

test('wasm 产物存在且导出所需函数', async (t) => {
  const module = await loadModule();
  if (!module) {
    t.skip('public/pkg 下没有 wasm 产物，先运行 npm run build:wasm');
    return;
  }
  assert.equal(typeof module.hitsoundNames, 'function');
  assert.equal(typeof module.hitsoundAsset, 'function');
  assert.equal(typeof module.hitsoundAssetCount, 'function');
  assert.equal(typeof module.hitsoundDefaults, 'function');
});

test('音效资源内嵌在 wasm 里且为合法 ogg', async (t) => {
  const module = await loadModule();
  if (!module) {
    t.skip('缺少 wasm 产物');
    return;
  }
  assert.equal(module.hitsoundAssetCount(), 36);
  for (const name of ['normal-hitnormal', 'soft-sliderslide', 'taiko-drum-hitclap', 'spinnerspin']) {
    const bytes = module.hitsoundAsset(name);
    assert.ok(bytes.length > 0, `${name} 没有内嵌资源`);
    // ogg 魔数：确认是真实资源而不是占位数据。
    assert.equal(String.fromCharCode(...bytes.slice(0, 4)), 'OggS', `${name} 不是 ogg`);
  }
  // 没有对应资源时返回空数组，宿主跳过即可。
  assert.equal(module.hitsoundAsset('不存在的音效').length, 0);
});

test('打击音默认值来自共享配置', async (t) => {
  const module = await loadModule();
  if (!module) {
    t.skip('缺少 wasm 产物');
    return;
  }
  // `assets/shared_config.yml` 四个模式的 mp4 默认值都是开启 + 50%。
  for (const mode of ['standard', 'taiko', 'catch', 'mania']) {
    const defaults = module.hitsoundDefaults(mode);
    assert.equal(defaults.enabled, true, `${mode} 默认应开启打击音`);
    assert.equal(defaults.volume, 50, `${mode} 默认音量应为 50`);
  }
  assert.equal(module.hitsoundDefaults('ctb').volume, 50);
  assert.throws(() => module.hitsoundDefaults('nope'));
});

test('standard 谱面按 timing point 的音效组展开样本名', async (t) => {
  const module = await loadModule();
  if (!module) {
    t.skip('缺少 wasm 产物');
    return;
  }
  const names = module.hitsoundNames(new TextEncoder().encode(standard_beatmap));
  // timing point 第 4 列为 2 = soft 音效组，因此普通打击音走 soft bank。
  assert.ok(names.includes('soft-hitnormal'), `缺少 soft-hitnormal：${names}`);
  assert.ok(names.includes('soft-slidertick'), `缺少 soft-slidertick：${names}`);
  assert.ok(names.includes('soft-sliderslide'), `缺少 soft-sliderslide：${names}`);
  assert.ok(names.includes('spinnerspin'), `缺少 spinnerspin：${names}`);
  assert.ok(names.includes('spinnerbonus'), `缺少 spinnerbonus：${names}`);
  // 裸名是回退查找，也应该登记。
  assert.ok(names.includes('hitnormal'), `缺少裸名回退：${names}`);
  // 关键回归：带音效组前缀的样本都必须真的内嵌在 wasm 里，否则 Web 端会整片静音。
  // 裸名（`hitnormal` 等）与转盘音的 `normal-` 变体是次级回退查找，本套皮肤不提供。
  for (const name of names) {
    if (!name.includes('-') || name.startsWith('normal-spinner')) continue;
    assert.ok(
      module.hitsoundAsset(name).length > 0,
      `样本 ${name} 没有内嵌资源（Web 端会静音）`,
    );
  }
});

test('taiko 谱面按采样音量选择音效组', async (t) => {
  const module = await loadModule();
  if (!module) {
    t.skip('缺少 wasm 产物');
    return;
  }
  const loud = module.hitsoundNames(new TextEncoder().encode(taiko_beatmap));
  // 采样音量 90 → drum 档。
  assert.ok(loud.includes('taiko-drum-hitnormal'), `缺少红音符音效：${loud}`);
  assert.ok(loud.includes('taiko-drum-hitclap'), `缺少蓝音符音效：${loud}`);

  const quiet = module.hitsoundNames(new TextEncoder().encode(taiko_soft_beatmap));
  // 采样音量 50 → soft 档。
  assert.ok(quiet.includes('taiko-soft-hitnormal'), `缺少低音量红音符音效：${quiet}`);
  assert.ok(quiet.includes('taiko-soft-hitclap'), `缺少低音量蓝音符音效：${quiet}`);
});
