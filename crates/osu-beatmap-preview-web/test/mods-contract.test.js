// Mod 面板契约测试：网页可勾选的 Mod 列表来自 core 的支持矩阵（`supportedMods`），
// 这里用 Node 直接跑 wasm 产物核对。
//
// 以前 `src/preview.js` 里手抄了一份 README 的支持表，PR #6 加入 HD/FL 之后没跟上，
// 网页就点不到这两个 Mod。清单现在只有一份（core），这份测试守住两件事：
// 四种模式都能选到 HD/FL，且列表里的每一项都是实时会话真正接受的形式。

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
  // 胶水代码把字符串/URL 一律交给 fetch，而 Node 的 fetch 不支持 file://。
  const wasm = await fsp.readFile(path.join(pkgDir, 'osu_beatmap_preview_wasm_bg.wasm'));
  await module.default({
    module_or_path: new Response(wasm, {
      headers: { 'Content-Type': 'application/wasm' },
    }),
  });
  return module;
}

/** 与 CLI README「GIF / MP4」列一致的期望表（转谱后的目标模式）。 */
const EXPECTED = {
  standard: ['EZ', 'HR', 'HD', 'FL', 'DA', 'TC', 'DT', 'HT', 'NC', 'DC'],
  taiko: ['EZ', 'HR', 'HD', 'FL', 'SW', 'CS', 'DT', 'HT', 'NC', 'DC'],
  catch: ['EZ', 'HR', 'HD', 'FL', 'DT', 'HT', 'NC', 'DC'],
  mania: [
    'HD', 'FL', 'CS', 'DT', 'HT', 'NC', 'DC',
    '1K', '2K', '3K', '4K', '5K', '6K', '7K', '8K', '9K', '10K',
    'DS', 'IN', 'HO',
  ],
};

test('supportedMods 导出齐全', async (t) => {
  const module = await loadModule();
  if (!module) {
    t.skip('public/pkg 下没有 wasm 产物，先运行 npm run build:wasm');
    return;
  }
  assert.equal(typeof module.supportedMods, 'function');
});

test('四种模式的 Mod 列表与支持矩阵一致，且都含 HD/FL/NC/DC', async (t) => {
  const module = await loadModule();
  if (!module) {
    t.skip('public/pkg 下没有 wasm 产物，先运行 npm run build:wasm');
    return;
  }
  for (const [mode, expected] of Object.entries(EXPECTED)) {
    const tokens = Array.from(module.supportedMods(mode));
    assert.deepEqual(tokens, expected, mode);
    // PR #6 之后实时链路四种模式都支持 HD/FL：面板必须能选到，否则又是「实现有、点不到」。
    for (const required of ['HD', 'FL', 'NC', 'DC']) {
      assert.ok(tokens.includes(required), `${mode} 缺少 ${required}`);
    }
  }
});

test('supportedMods 接受别名，未知模式报错', async (t) => {
  const module = await loadModule();
  if (!module) {
    t.skip('public/pkg 下没有 wasm 产物，先运行 npm run build:wasm');
    return;
  }
  assert.deepEqual(
    Array.from(module.supportedMods(' std ')),
    EXPECTED.standard,
  );
  assert.deepEqual(Array.from(module.supportedMods('CTB')), EXPECTED.catch);
  assert.throws(() => module.supportedMods('不存在的模式'));
});
