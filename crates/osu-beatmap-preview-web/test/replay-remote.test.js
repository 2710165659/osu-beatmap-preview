// 浏览器在线输入验证不依赖 Vue 或网络；响应字节随后仍交给 WASM 校验。
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { onlineReplayQuery, scoreIdFromInput, fetchOnlineReplay } from '../src/replay/remote.js';
import { resolveReplaySource } from '../src/replay/load.js';

test('成绩输入支持新式官方链接，拒绝旧式或伪造链接', () => {
  assert.equal(scoreIdFromInput(' 6086051920 '), '6086051920');
  assert.equal(scoreIdFromInput('https://osu.ppy.sh/scores/6086051920?mode=osu'), '6086051920');
  for (const input of ['0', '-1', 'https://evil.test/scores/1', 'https://osu.ppy.sh/scores/osu/1', 'https://user@osu.ppy.sh/scores/1', 'https://osu.ppy.sh.evil/scores/1']) assert.throws(() => scoreIdFromInput(input));
});

test('来源切换只发送当前来源参数，不带旧成绩或玩家信息', () => {
  const state = { replayMethod: 'score', replayScoreId: '10', replayUserValue: 'old user', replayBid: '42', replaySelection: 'recent', replayMode: '', replayIncludeFails: false };
  assert.equal(onlineReplayQuery(state).toString(), 'scoreId=10');
  state.replayMethod = 'player';
  assert.equal(onlineReplayQuery(state).toString(), 'user=old+user&bid=42&selection=recent');
  state.replayMethod = 'latest';
  state.replaySelection = 'best';
  assert.equal(onlineReplayQuery(state).toString(), 'user=old+user');
  state.replayMode = 'osu'; state.replayIncludeFails = true;
  assert.equal(onlineReplayQuery(state).toString(), 'user=old+user&mode=osu&includeFails=1');
  state.replayMethod = 'player';
  assert.equal(onlineReplayQuery(state).toString(), 'user=old+user&bid=42&selection=best&mode=osu');
  state.replaySelection = 'recent';
  assert.equal(onlineReplayQuery(state).get('includeFails'), '1');
  state.replayMethod = 'score';
  assert.equal(onlineReplayQuery(state).toString(), 'scoreId=10');
  state.replayMethod = 'file'; assert.equal(onlineReplayQuery(state), null);
});

test('玩家来源校验用户名、谱面和官方模式，不指定谱面只允许最近游玩入口', () => {
  const state = { replayMethod: 'latest', replayUserValue: '123', replayMode: '' };
  assert.equal(onlineReplayQuery(state).toString(), 'user=123');
  assert.throws(() => onlineReplayQuery({ ...state, replayUserValue: ' ' }), /用户名/);
  assert.throws(() => onlineReplayQuery({ ...state, replayMode: 'ctb' }), /模式/);
  assert.throws(() => onlineReplayQuery({ ...state, replayMethod: 'player', replayBid: '' }), /谱面 ID/);
});

test('下载回放后核验成绩身份与 MD5，直接使用已知 BID 下载谱面包', async () => {
  const checksum = 'a'.repeat(32);
  const wasm = { replayInfo: () => ({ checksum }), beatmapInfo: () => ({ beatmapId: 42 }) };
  const headers = { 'x-osu-score-id': '10', 'x-osu-checksum': checksum, 'x-osu-beatmap-id': '42' };
  const query = new URLSearchParams('scoreId=10');
  const signal = new AbortController().signal;
  const replay = await fetchOnlineReplay(query, wasm, signal, async (url, options) => {
    assert.equal(url, '/resource/replay?scoreId=10'); assert.equal(options.signal, signal);
    return new Response(new Uint8Array([1, 2]), { headers });
  });
  assert.equal(replay.scoreId, '10');
  await resolveReplaySource({ replay, wasm, lookup: () => assert.fail('不能重复查 MD5'), download: async (bid) => { assert.equal(bid, '42'); return new Uint8Array([3]); } });
  await assert.rejects(fetchOnlineReplay(query, wasm, signal, async () => new Response(new Uint8Array([1]), { headers: { ...headers, 'x-osu-checksum': 'b'.repeat(32) } })), /BeatmapMismatch/);
  await assert.rejects(fetchOnlineReplay(query, wasm, signal, async () => new Response(new Uint8Array([1]), { headers: { ...headers, 'x-osu-score-id': '11' } })), /BeatmapMismatch/);
});

test('下载错误和不兼容回放原样报错，无音画会话回退', async () => {
  const query = new URLSearchParams('scoreId=10');
  await assert.rejects(fetchOnlineReplay(query, {}, undefined, async () => new Response('没有回放', { status: 404 })), /没有回放/);
  await assert.rejects(fetchOnlineReplay(query, { replayInfo: () => { throw new Error('UnsupportedReplayVariant: AL'); } }, undefined, async () => new Response(new Uint8Array([1]))), /UnsupportedReplayVariant/);
});
