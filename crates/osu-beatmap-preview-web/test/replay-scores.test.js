// 来源与官方成绩查询契约全部注入响应，不访问网络或读取认证环境。
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { replayQuery, selectReplayScore } from '../backend/replay-scores.js';
const score = (id = 10, extra = {}) => ({ id, user_id: 7, beatmap_id: 2, ruleset_id: 0, has_replay: true, ended_at: '2026-01-01T00:00:00Z', beatmap: { id: 2, beatmapset_id: 3, checksum: 'a'.repeat(32) }, ...extra });

test('在线来源严格互斥，拒绝空值、重复参数和非法 ID', () => {
  for (const raw of ['', 'scoreId=1&user=x', 'scoreId=1&bid=2', 'scoreId=1&selection=best', 'user=x&userId=7&bid=2', 'user=x&selection=best', 'userId=7&bid=2', 'user=', 'user=x&bid=', 'scoreId=1&scoreId=2', 'scoreId=18446744073709551616', 'user=x&bid=2&selection=pp', 'scoreId=1&mode=osu', 'scoreId=1&includeFails=0', 'user=x&mode=ctb', 'user=x&mode=osu&mode=mania', 'user=x&includeFails=true', 'user=x&includeFails=0&includeFails=1', 'user=x&bid=2&includeFails=1']) assert.throws(() => replayQuery(new URLSearchParams(raw)), undefined, raw);
  assert.deepEqual(replayQuery(new URLSearchParams('scoreId=10')), { scoreId: '10' });
  assert.equal(replayQuery(new URLSearchParams('user=123&bid=2')).selection, 'best');
  assert.deepEqual(replayQuery(new URLSearchParams('user=123')), { user: '123', bid: null, selection: 'recent', mode: null, includeFails: false });
});

test('最高分按指定玩家和谱面查询，数字用户名不被当作 ID', async () => {
  const calls = [];
  const result = await selectReplayScore({ get: async (path) => {
    calls.push(path);
    if (path === 'beatmaps/2') return { id: 2, mode: 'osu' };
    return path.startsWith('users/') ? { id: 7 } : { score: score() };
  } }, replayQuery(new URLSearchParams('user=123&bid=2')));
  assert.deepEqual(calls, ['beatmaps/2', 'users/@123/osu', 'beatmaps/2/scores/users/7?mode=osu&legacy_only=0']);
  assert.deepEqual(result, { scoreId: '10', bid: '2', setId: '3', checksum: 'a'.repeat(32) });
});

test('近期查询分页并按绝对时间选择，不跳过没有回放的最新成绩', async () => {
  const calls = [];
  const api = { get: async (path) => {
    calls.push(path);
    if (path === 'users/@x/osu') return { id: 7 };
    return path.endsWith('offset=0') ? Array.from({ length: 100 }, () => score(1, { beatmap_id: 999 })) : [
      score(11, { ended_at: '2026-01-01T00:30:00+02:00' }),
      score(12, { ended_at: '2026-01-01T01:00:00+01:00', has_replay: false }),
    ];
  } };
  await assert.rejects(selectReplayScore(api, replayQuery(new URLSearchParams('user=x&bid=2&selection=recent&includeFails=1&mode=osu'))), /没有可下载回放/);
  assert.equal(calls.length, 3);
  assert.ok(calls[2].endsWith('offset=100'));
  assert.ok(calls.slice(1).every((p) => p.includes('include_fails=1')));
});

test('近期扫描有上限，缺失不回退到最高分', async () => {
  let calls = 0;
  const api = { get: async (path) => {
    if (path === 'users/@x/osu') return { id: 7 };
    assert.ok(path.includes('/scores/recent?')); calls++;
    return Array.from({ length: 100 }, () => score(1, { beatmap_id: 999 }));
  } };
  await assert.rejects(selectReplayScore(api, replayQuery(new URLSearchParams('user=x&bid=2&selection=recent&mode=osu'))), /不会回退/);
  assert.equal(calls, 10);
});

test('拒绝错误成绩身份、其它模式和无回放，不替换成其它成绩', async () => {
  for (const [value, pattern] of [
    [score(11), /成绩 ID 不符/],
    [score(10, { ruleset_id: 4 }), /Standard、Taiko、Catch 和 Mania/],
    [score(10, { has_replay: false }), /没有可下载回放/],
    [score(10, { beatmap_id: 9 }), /谱面信息/],
  ]) await assert.rejects(selectReplayScore({ get: async () => value }, { scoreId: '10' }), pattern);
  await assert.rejects(selectReplayScore({ get: async (path) => path.startsWith('users/') ? { id: 7 } : { score: score(10, { user_id: 9 }) } }, replayQuery(new URLSearchParams('user=x&bid=2&mode=osu'))), /玩家或谱面/);
});

test('最近游玩省略模式和谱面，默认排除失败，只查询最近一条', async () => {
  const calls = [];
  const result = await selectReplayScore({ get: async (path) => {
    calls.push(path);
    return path === 'users/@123' ? { id: 7 } : [score()];
  } }, replayQuery(new URLSearchParams('user=123')));
  assert.deepEqual(calls, ['users/@123', 'users/7/scores/recent?include_fails=0&legacy_only=0&limit=1&offset=0']);
  assert.equal(result.bid, '2');
});

test('最近游玩传递官方模式和失败开关，不改选不兼容或无回放成绩', async () => {
  for (const [mode, extra, pattern] of [
    ['osu', { has_replay: false }, /没有可下载回放/],
    ['mania', { ruleset_id: 3, has_replay: false }, /没有可下载回放/],
  ]) {
    const calls = [];
    await assert.rejects(selectReplayScore({ get: async (path) => {
      calls.push(path);
      return path === `users/@x/${mode}` ? { id: 7 } : [score(10, extra)];
    } }, replayQuery(new URLSearchParams(`user=x&mode=${mode}&includeFails=1`))), pattern);
    assert.deepEqual(calls, [`users/@x/${mode}`, `users/7/scores/recent?mode=${mode}&include_fails=1&legacy_only=0&limit=1&offset=0`]);
  }
});

test('最近游玩没有记录或返回错误玩家时明确报错', async () => {
  for (const [entries, pattern] of [[[], /没有符合条件/], [[score(10, { user_id: 9 })], /玩家或谱面/]]) {
    await assert.rejects(selectReplayScore({ get: async (path) => path === 'users/@x' ? { id: 7 } : entries }, replayQuery(new URLSearchParams('user=x'))), pattern);
  }
});


test('玩家加谱面未指定模式时按原谱查询，显式模式可定位转谱成绩', async () => {
  for (const [mode, params] of [['mania', '&mode=mania'], ['mania', '']]) {
    const calls = [];
    const result = await selectReplayScore({ get: async (path) => {
      calls.push(path);
      if (path === 'beatmaps/2') return { id: 2, mode: 'mania' };
      if (path === `users/@x/${mode}`) return { id: 7 };
      return { score: score(10, { ruleset_id: 3 }) };
    } }, replayQuery(new URLSearchParams(`user=x&bid=2${params}`)));
    assert.equal(result.scoreId, '10');
    assert.deepEqual(calls, [...(params ? [] : ['beatmaps/2']), `users/@x/${mode}`, `beatmaps/2/scores/users/7?mode=${mode}&legacy_only=0`]);
  }
});

test('Catch 成绩可按成绩 ID 或 fruits 最近游玩定位', async () => {
  const value = score(10, { ruleset_id: 2 });
  assert.equal((await selectReplayScore({ get: async () => value }, { scoreId: '10' })).scoreId, '10');
  const calls = [];
  const result = await selectReplayScore({ get: async (p) => {
    calls.push(p); return p === 'users/@x/fruits' ? { id: 7 } : [value];
  } }, replayQuery(new URLSearchParams('user=x&mode=fruits&includeFails=1')));
  assert.equal(result.scoreId, '10');
  assert.deepEqual(calls, ['users/@x/fruits', 'users/7/scores/recent?mode=fruits&include_fails=1&legacy_only=0&limit=1&offset=0']);
});
