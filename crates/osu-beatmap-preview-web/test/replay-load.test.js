// 回放加载的资源选择与失败边界使用注入依赖验证，不请求网络。
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createReplayInput, readReplayFile, resolveReplaySource } from '../src/replay/load.js';
const checksum = 'a'.repeat(32);
const replay = { info: { checksum }, bytes: new Uint8Array([1]) };

test('本地 OSZ 以 checksum 绑定，不下载也不按 BID 选错难度', async () => {
  const bytes = new Uint8Array([2]);
  const result = await resolveReplaySource({ replay, localBytes: bytes, localKind: 'osz', wasm: { beatmapInfo: (raw, selector) => { assert.equal(raw, bytes); assert.deepEqual(selector, { checksum }); return { version: 'Hard' }; } }, lookup: () => assert.fail('不应查询'), download: () => assert.fail('不应下载') });
  assert.equal(result.bytes, bytes);
});

test('提供不匹配谱面立即失败，不替换成自动下载谱面', async () => {
  await assert.rejects(resolveReplaySource({ replay, localBytes: new Uint8Array([2]), localKind: 'osz', wasm: { beatmapInfo() { throw new Error('BeatmapMismatch'); } }, lookup: () => assert.fail(), download: () => assert.fail() }), /BeatmapMismatch/);
});

test('OSR 单文件经官方查谱下载，再对包内原字节验证 MD5', async () => {
  const signal = new AbortController().signal;
  const result = await resolveReplaySource({ replay, signal, lookup: async (value, passed) => { assert.equal(value, checksum); assert.equal(passed, signal); return { bid: '42' }; }, download: async (bid, options) => { assert.equal(bid, '42'); assert.equal(options.signal, signal); return new Uint8Array([3]); }, wasm: { beatmapInfo(raw, selector) { assert.equal(raw[0], 3); assert.equal(selector.checksum, checksum); return {}; } } });
  assert.equal(result.bytes[0], 3);
});

test('本地 OSU 仍下载完整包以提供背景和音乐；下载失败上报', async () => {
  await assert.rejects(resolveReplaySource({ replay, localBytes: new Uint8Array([2]), localKind: 'osu', wasm: { beatmapInfo: () => ({ beatmapId: 123 }) }, lookup: () => assert.fail('已有 BID'), download: async (bid) => { assert.equal(bid, '123'); throw new Error('下载失败'); } }), /下载失败/);
});

test('回放选择限制后缀和大小，兼容性错误传给界面', async () => {
  await assert.rejects(readReplayFile({ name: 'x.osu', size: 10 }, {}), /osr/);
  await assert.rejects(readReplayFile({ name: 'x.osr', size: 65 * 1024 * 1024 }, {}), /64 MiB/);
  await assert.rejects(readReplayFile({ name: 'x.osr', size: 1, arrayBuffer: async () => new ArrayBuffer(1) }, { replayInfo() { throw new Error('UnsupportedReplayVariant: AL'); } }), /AL/);
});


test('清除或新选回放后旧解析不能覆盖输入或报错', async () => {
  const state = { convert: 'mania' };
  const input = createReplayInput(state, async () => ({ replayInfo: () => ({ checksum }) }));
  let rejectOld;
  const old = input.select({ name: 'old.osr', size: 1, arrayBuffer: () => new Promise((resolve, reject) => { rejectOld = reject; }) });
  // 等异步 WASM 载入进入旧文件读取，再选择新文件。
  await new Promise((resolve) => setImmediate(resolve));
  await input.select({ name: 'new.osr', size: 1, arrayBuffer: async () => new ArrayBuffer(1) });
  rejectOld(new Error('旧请求失败'));
  await old;
  assert.equal(state.replayFileName, 'new.osr');
  assert.equal(state.convert, '');
  assert.equal(input.current().name, 'new.osr');
  input.clear();
  assert.equal(input.current(), null);
  assert.equal(state.replaySelecting, false);
});
