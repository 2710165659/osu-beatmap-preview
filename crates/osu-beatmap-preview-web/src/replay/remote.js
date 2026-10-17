// 在线来源只传定位参数，回放字节返回浏览器后仍执行 WASM 兼容检查和 MD5 绑定。
export function scoreIdFromInput(raw) {
  const text = String(raw ?? '').trim();
  if (/^[1-9]\d*$/.test(text)) return text;
  let url;
  try { url = new URL(text); } catch { throw new Error('请输入正整数成绩 ID 或官方成绩链接'); }
  const match = url.pathname.match(/^\/scores\/([1-9]\d*)\/?$/);
  if (url.protocol !== 'https:' || url.hostname !== 'osu.ppy.sh' || url.port || url.username || url.password || !match) throw new Error('仅支持 https://osu.ppy.sh/scores/<成绩ID>；旧版成绩链接暂不支持');
  return match[1];
}

export function onlineReplayQuery(state) {
  if (state.replayMethod === 'file') return null;
  if (state.replayMethod === 'score') return new URLSearchParams({ scoreId: scoreIdFromInput(state.replayScoreId) });
  if (!['player', 'latest'].includes(state.replayMethod)) throw new Error('无效回放来源');
  const user = state.replayUserValue.trim();
  if (!user || user.length > 256) throw new Error('请输入玩家用户名');
  const query = new URLSearchParams({ user });
  if (state.replayMethod === 'player') {
    const bid = state.replayBid.trim();
    if (!/^[1-9]\d*$/.test(bid)) throw new Error('请输入正整数谱面 ID');
    if (!['best', 'recent'].includes(state.replaySelection)) throw new Error('成绩选择必须是 best 或 recent');
    query.set('bid', bid);
    query.set('selection', state.replaySelection);
  }
  const mode = state.replayMode ?? '';
  if (mode && !['osu', 'taiko', 'fruits', 'mania'].includes(mode)) throw new Error('无效查询模式');
  if (mode) query.set('mode', mode);
  // 隐藏的谱面、成绩选择与失败开关不参与当前来源，避免切换后混入旧参数。
  if ((state.replayMethod === 'latest' || state.replaySelection === 'recent') && state.replayIncludeFails) query.set('includeFails', '1');
  return query;
}

export async function fetchOnlineReplay(query, wasm, signal, fetchImpl = fetch) {
  const response = await fetchImpl(`/resource/replay?${query}`, { signal });
  if (!response.ok) throw new Error(await response.text());
  const limit = 64 * 1024 * 1024;
  if (Number(response.headers.get('content-length')) > limit) throw new Error('回放超过 64 MiB');
  const chunks = [];
  let size = 0;
  const reader = response.body.getReader();
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.length;
      if (size > limit) throw new Error('回放超过 64 MiB');
      chunks.push(value);
    }
  } finally { await reader.cancel().catch(() => {}); reader.releaseLock(); }
  const bytes = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.length; }
  const info = wasm.replayInfo(bytes);
  const checksum = response.headers.get('x-osu-checksum');
  const bid = response.headers.get('x-osu-beatmap-id');
  const scoreId = response.headers.get('x-osu-score-id');
  if (!/^[a-f0-9]{32}$/i.test(checksum ?? '') || info.checksum.toLowerCase() !== checksum.toLowerCase() || !/^[1-9]\d*$/.test(bid ?? '') || !/^[1-9]\d*$/.test(scoreId ?? '') || (query.has('scoreId') && query.get('scoreId') !== scoreId) || (query.has('bid') && query.get('bid') !== bid)) throw new Error('BeatmapMismatch: 在线成绩与回放信息不符');
  return { bytes, info, name: `score-${scoreId}.osr`, location: { bid }, scoreId };
}
