// 回放下载使用本地样本和临时缓存，验证并发、缓存绕过与失败清理。
import assert from 'node:assert/strict';
import { test } from 'node:test';
import fsp from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { createReplayDownload } from '../backend/replay-download.js';
const bytes = await fsp.readFile(new URL('./fixtures/replay-standard.osr', import.meta.url));
const checksum = bytes.subarray(7, 39).toString('ascii');
const entry = { id: 10, user_id: 7, beatmap_id: 42, ruleset_id: 0, has_replay: true, beatmap: { id: 42, beatmapset_id: 3, checksum } };

test('同成绩并发共用下载，文件缓存可复用且 noCache 可绕过', async () => {
  const cacheDir = await fsp.mkdtemp(path.join(os.tmpdir(), 'osu-replay-download-'));
  try {
    let downloads = 0;
    const api = { get: async () => entry, response: async () => { downloads++; return new Response(bytes); } };
    const load = createReplayDownload({ api, cacheDir });
    const results = await Promise.all([load({ scoreId: '10' }), load({ scoreId: '10' })]);
    assert.equal(downloads, 1);
    assert.deepEqual(results[0].bytes, bytes);
    await load({ scoreId: '10' }); assert.equal(downloads, 1);
    await createReplayDownload({ api, cacheDir, noCache: true })({ scoreId: '10' }); assert.equal(downloads, 2);
    await fsp.writeFile(path.join(cacheDir, 'osr-download-cache/10.osr'), 'broken');
    await load({ scoreId: '10' }); assert.equal(downloads, 3);
  } finally { await fsp.rm(cacheDir, { recursive: true, force: true }); }
});

test('Taiko 回放头部可下载并复用缓存', async () => {
  const cacheDir = await fsp.mkdtemp(path.join(os.tmpdir(), 'osu-replay-download-taiko-'));
  try {
    const taikoBytes = Buffer.from(bytes);
    taikoBytes[0] = 1;
    let downloads = 0;
    const api = { get: async () => ({ ...entry, ruleset_id: 1 }), response: async () => { downloads++; return new Response(taikoBytes); } };
    const load = createReplayDownload({ api, cacheDir });
    const result = await load({ scoreId: '10' });
    assert.deepEqual(result.bytes, taikoBytes);
    await load({ scoreId: '10' });
    assert.equal(downloads, 1);
  } finally { await fsp.rm(cacheDir, { recursive: true, force: true }); }
});

test('下载损坏、不支持的模式、MD5 不符、超限和网络错误不会成为缓存', async () => {
  const cacheDir = await fsp.mkdtemp(path.join(os.tmpdir(), 'osu-replay-reject-'));
  try {
    for (const mode of [0, 1]) for (const [response, pattern] of [
      [async () => new Response('not a replay'), /MD5/],
      ...[2, 3, 255].map((unsupported) => [async () => new Response(Buffer.from(bytes).fill(unsupported, 0, 1)), /MD5/]),
      [async () => new Response(Buffer.from(bytes).fill(mode, 0, 1).fill(97, 7, 39)), /MD5/],
      [async () => new Response(bytes, { headers: { 'content-length': 65 * 1024 * 1024 } }), /64 MiB/],
      [async () => { throw new Error('network failure'); }, /network failure/],
    ]) {
      const load = createReplayDownload({ api: { get: async () => ({ ...entry, ruleset_id: mode }), response }, cacheDir });
      await assert.rejects(load({ scoreId: '10' }), pattern);
      await assert.rejects(fsp.access(path.join(cacheDir, 'osr-download-cache/10.osr')));
    }
  } finally { await fsp.rm(cacheDir, { recursive: true, force: true }); }
});
