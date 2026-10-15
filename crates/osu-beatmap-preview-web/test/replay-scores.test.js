// 来源与官方成绩查询契约全部注入响应，不访问网络或读取认证环境。
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { replayQuery, selectReplayScore } from '../backend/replay-scores.js';
const score = (id = 10, extra = {}) => ({ id, user_id: 7, beatmap_id: 2, ruleset_id: 0, has_replay: true, ended_at: '2026-01-01T00:00:00Z', beatmap: { id: 2, beatmapset_id: 3, checksum: 'a'.repeat(32) }, ...extra });

test('在线来源严格互斥，拒绝空值、重复参数和非法 ID', () => {
  for (const raw of ['', 'scoreId=1&user=x', 'scoreId=1&bid=2', 'scoreId=1&selection=best', 'user=x&userId=7&bid=2', 'user=x', 'userId=0&bid=2', 'scoreId=1&scoreId=2', 'scoreId=18446744073709551616', 'user=x&bid=2&selection=pp']) assert.throws(() => replayQuery(new URLSearchParams(raw)), undefined, raw);
  assert.deepEqual(replayQuery(new URLSearchParams('scoreId=10')), { scoreId: '10' });
  assert.equal(replayQuery(new URLSearchParams('user=123&bid=2')).selection, 'best');
});

test('最高分按指定玩家和谱面查询，数字用户名不被当作 ID', async () => {
  const calls = [];
  const result = await selectReplayScore({ get: async (path) => {
    calls.push(path);
    return path.startsWith('users/') ? { id: 7 } : { score: score() };
  } }, replayQuery(new URLSearchParams('user=123&bid=2')));
  assert.deepEqual(calls, ['users/@123/osu', 'beatmaps/2/scores/users/7?mode=osu&legacy_only=0']);
  assert.deepEqual(result, { scoreId: '10', bid: '2', setId: '3', checksum: 'a'.repeat(32) });
});

test('近期查询分页并按绝对时间选择，不跳过没有回放的最新成绩', async () => {
  const calls = [];
  const api = { get: async (path) => {
    calls.push(path);
    return path.endsWith('offset=0') ? Array.from({ length: 100 }, () => score(1, { beatmap_id: 999 })) : [
      score(11, { ended_at: '2026-01-01T00:30:00+02:00' }),
      score(12, { ended_at: '2026-01-01T01:00:00+01:00', has_replay: false }),
    ];
  } };
  await assert.rejects(selectReplayScore(api, replayQuery(new URLSearchParams('userId=7&bid=2&selection=recent'))), /没有可下载回放/);
  assert.equal(calls.length, 2);
  assert.ok(calls[1].endsWith('offset=100'));
  assert.ok(calls.every((p) => p.includes('include_fails=1')));
});

test('近期扫描有上限，缺失不回退到最高分', async () => {
  let calls = 0;
  const api = { get: async (path) => {
    assert.ok(path.includes('/scores/recent?')); calls++;
    return Array.from({ length: 100 }, () => score(1, { beatmap_id: 999 }));
  } };
  await assert.rejects(selectReplayScore(api, replayQuery(new URLSearchParams('userId=7&bid=2&selection=recent'))), /不会回退/);
  assert.equal(calls, 10);
});

test('拒绝错误成绩身份、其它模式和无回放，不替换成其它成绩', async () => {
  for (const [value, pattern] of [
    [score(11), /成绩 ID 不符/],
    [score(10, { ruleset_id: 1 }), /Standard/],
    [score(10, { has_replay: false }), /没有可下载回放/],
    [score(10, { beatmap_id: 9 }), /谱面信息/],
  ]) await assert.rejects(selectReplayScore({ get: async () => value }, { scoreId: '10' }), pattern);
  await assert.rejects(selectReplayScore({ get: async () => ({ score: score(10, { user_id: 9 }) }) }, { userId: '7', bid: '2', selection: 'best' }), /玩家或谱面/);
});
