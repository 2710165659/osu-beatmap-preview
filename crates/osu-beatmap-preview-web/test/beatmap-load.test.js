import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createLocalInputs, resolveBeatmapSource } from '../src/beatmap/load.js';

test('普通预览支持 OSU、OSZ、双文件与在线下载，BID 优先于难度选择', async () => {
  const osu = new Uint8Array([1]);
  const osz = new Uint8Array([2]);
  const wasm = { beatmapInfo: () => ({ beatmapId: 42 }) };
  const noDownload = () => assert.fail('本地资源不应下载');
  const single = await resolveBeatmapSource({ wasm, osuBytes: osu, download: noDownload });
  assert.equal(single.bytes, osu);
  assert.equal(single.hasMusic, false);
  const archive = await resolveBeatmapSource({ wasm, oszBytes: osz, bid: '42', difficulty: 'wrong.osu', download: noDownload });
  assert.deepEqual(archive.selector, { bid: '42' });
  const combined = await resolveBeatmapSource({ wasm, osuBytes: osu, oszBytes: osz, difficulty: 'ignored.osu', download: noDownload });
  assert.equal(combined.bytes, osz);
  assert.deepEqual(combined.selector, { osuBytes: osu });
  assert.equal(combined.hasMusic, true);
  const online = await resolveBeatmapSource({ wasm, bid: '42', download: async (bid) => { assert.equal(bid, '42'); return osz; } });
  assert.equal(online.bytes, osz);
  await assert.rejects(resolveBeatmapSource({ wasm, bid: '0', download: noDownload }), /正整数/);
});

test('独立文件可同时保留，清除与过期解析不会覆盖新选择', async () => {
  const state = { bid: '' };
  const controller = createLocalInputs(state, async () => ({ beatmapInfo: () => ({ difficulties: [{ entry: 'a.osu' }] }) }));
  const file = (name, value) => ({ name, arrayBuffer: async () => new Uint8Array([value]).buffer });
  await controller.select('osu', file('a.osu', 1));
  await controller.select('osz', file('a.osz', 2));
  assert.equal(controller.current().osuBytes[0], 1);
  assert.equal(controller.current().oszBytes[0], 2);
  let finish;
  const pending = controller.select('osz', { name: 'old.osz', arrayBuffer: () => new Promise((resolve) => { finish = resolve; }) });
  controller.clear('osz');
  await controller.select('osz', file('new.osz', 3));
  finish(new Uint8Array([4]).buffer);
  await pending;
  assert.equal(controller.current().oszBytes[0], 3);
  controller.clear('osu');
  assert.equal(controller.current().osuBytes, null);
  assert.equal(state.localOszName, 'new.osz');
  assert.equal(state.localOszSelecting, false);
});
