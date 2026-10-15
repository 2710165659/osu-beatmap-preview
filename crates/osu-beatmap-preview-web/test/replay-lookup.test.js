// 官方 API 契约使用模拟 fetch 与私有环境，不读取开发者凭据。
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createReplayLookup } from '../backend/replay-lookup.js';
const checksum = 'a'.repeat(32);
const reply = (value) => ({ ok: true, json: async () => value });

test('checksum 非法和无凭据时不发出网络请求', async () => {
  const lookup = createReplayLookup({ env: {}, fetchImpl: () => assert.fail() });
  await assert.rejects(lookup('../wrong'), /MD5/);
  await assert.rejects(lookup(checksum), /OSU_API_TOKEN/);
});

test('官方 OAuth 并发单飞且缓存 token，返回只包含谱面定位', async () => {
  let oauth = 0;
  const lookup = createReplayLookup({ env: { OSU_CLIENT_ID: '1', OSU_CLIENT_SECRET: 'private' }, fetchImpl: async (url, options) => {
    if (url.endsWith('/oauth/token')) { oauth++; assert.equal(JSON.parse(options.body).scope, 'public'); return reply({ access_token: 'private-token', expires_in: 3600 }); }
    assert.equal(options.headers.Authorization, 'Bearer private-token');
    return reply({ id: 42, beatmapset_id: 12, checksum, secret: 'never forward' });
  } });
  const result = await Promise.all([lookup(checksum), lookup(checksum)]);
  assert.equal(oauth, 1);
  assert.deepEqual(result[0], { bid: '42', setId: '12', checksum });
  await lookup(checksum);
  assert.equal(oauth, 1);
});

test('官方 hash 不符与失败状态拒绝，错误不包含凭据', async () => {
  const lookup = createReplayLookup({ env: { OSU_API_TOKEN: 'private' }, fetchImpl: async () => reply({ id: 42, beatmapset_id: 12, checksum: 'b'.repeat(32) }) });
  await assert.rejects(lookup(checksum), /不符/);
  const failure = createReplayLookup({ env: { OSU_API_TOKEN: 'private' }, fetchImpl: async () => { throw new Error('private'); } });
  await assert.rejects(failure(checksum), (error) => error.message.includes('网络') && !error.message.includes('private'));
});
