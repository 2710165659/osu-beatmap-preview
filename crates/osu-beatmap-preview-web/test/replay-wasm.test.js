// 独立合成 OSR/OSU 验证真实 WASM 导出；资源仅位于 test，不进入发布包。
import assert from 'node:assert/strict';
import { test } from 'node:test';
import fsp from 'node:fs/promises';
import { fileURLToPath, pathToFileURL } from 'node:url';
import path from 'node:path';
const here = path.dirname(fileURLToPath(import.meta.url));

async function moduleOrSkip(t) {
  const glue = path.resolve(here, '../public/pkg/osu_beatmap_preview_wasm.js');
  try { await fsp.access(glue); } catch { t.skip('先运行 npm run build:wasm'); return null; }
  const wasm = await import(pathToFileURL(glue).href);
  await wasm.default({ module_or_path: new Response(await fsp.readFile(glue.replace('.js', '_bg.wasm')), { headers: { 'Content-Type': 'application/wasm' } }) });
  return wasm;
}

test('真实 WASM 解析 LZMA 回放与 MD5 选谱，拒绝格式损坏', async (t) => {
  const wasm = await moduleOrSkip(t);
  if (!wasm) return;
  const replay = new Uint8Array(await fsp.readFile(path.join(here, 'fixtures/replay-standard.osr')));
  const map = new Uint8Array(await fsp.readFile(path.join(here, 'fixtures/replay-standard.osu')));
  const info = wasm.replayInfo(replay);
  assert.equal(info.version, 30000016);
  assert.equal(wasm.beatmapInfo(map, { checksum: info.checksum }).beatmapId, 42);
  assert.throws(() => wasm.beatmapInfo(new Uint8Array([...map, 10]), { checksum: info.checksum }), /BeatmapMismatch/);
  assert.throws(() => wasm.replayInfo(replay.slice(0, 30)), /InvalidReplay/);
  assert.equal(typeof wasm.WebGpuSession.prototype.replayState, 'function');
});

test('WASM 格式成功解析后仍拒绝未验证 Mod 声明', async (t) => {
  const wasm = await moduleOrSkip(t);
  if (!wasm) return;
  const replay = new Uint8Array(await fsp.readFile(path.join(here, 'fixtures/replay-standard.osr')));
  // 三个 osu! 字符串之后依次是 12 字节计数、4 字节分数、2 字节连击、1 字节布尔。
  let cursor = 5;
  for (let field = 0; field < 3; field++) {
    assert.equal(replay[cursor++], 0x0b);
    let size = 0, shift = 0, value;
    do { value = replay[cursor++]; size += (value & 127) * 2 ** shift; shift += 7; } while (value & 128);
    cursor += size;
  }
  new DataView(replay.buffer).setUint32(cursor + 19, 1, true);
  assert.throws(() => wasm.replayInfo(replay), /UnsupportedReplayVariant/);
});

test('真实 WASM 接受 stable 自动 CL 和 tachyon 显式 CL，不限制分数版本', async (t) => {
  const wasm = await moduleOrSkip(t);
  if (!wasm) return;
  const map = new Uint8Array(await fsp.readFile(path.join(here, 'fixtures/replay-classic.osu')));
  for (const [file, expectedVersion] of [['replay-classic.osr', 20200101], ['replay-classic-lazer.osr', 30000019]]) {
    const replay = new Uint8Array(await fsp.readFile(path.join(here, 'fixtures', file)));
    const info = wasm.replayInfo(replay);
    assert.equal(info.version, expectedVersion);
    assert.equal(wasm.beatmapInfo(map, { checksum: info.checksum }).beatmapId, 43);
    // 新的分数版本仍沿用同一容器：不要求先更新客户端或分数版本白名单。
    if (expectedVersion > 30000000) {
      new DataView(replay.buffer).setInt32(1, 40000000, true);
      assert.equal(wasm.replayInfo(replay).version, 40000000);
    }
  }
});
