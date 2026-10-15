// 成绩选择只负责定位一条记录；下载和渲染失败时不改选其它成绩。
export function replayQuery(params) {
  const scoreId = params.get('scoreId');
  const user = params.get('user');
  const userId = params.get('userId');
  const bid = params.get('bid');
  const selection = params.get('selection');
  const validId = (value) => typeof value === 'string' && /^[1-9]\d*$/.test(value) && BigInt(value) <= 18446744073709551615n;
  for (const key of ['scoreId', 'user', 'userId', 'bid', 'selection']) {
    if (params.getAll(key).length > 1) throw new Error('回放来源参数不可重复');
  }
  if (scoreId !== null) {
    if (!validId(scoreId) || [user, userId, bid, selection].some((x) => x !== null)) throw new Error('成绩 ID 与玩家查询互斥，需要正整数成绩 ID');
    return { scoreId };
  }
  if ((user !== null) === (userId !== null) || !validId(bid) || (userId !== null && !validId(userId)) || (user !== null && (!user.trim() || user.length > 256))) throw new Error('需要玩家名或玩家 ID（二选一），以及正整数谱面 ID');
  if (selection !== null && !['best', 'recent'].includes(selection)) throw new Error('成绩选择必须是 best 或 recent');
  return { user, userId, bid, selection: selection ?? 'best' };
}

export async function selectReplayScore(api, query) {
  const request = (path) => api.get(path);
  let score;
  let userId;
  if (query.scoreId) {
    score = await request(`scores/${query.scoreId}`);
    if (String(score.id) !== query.scoreId) throw new Error('官方返回的成绩 ID 不符');
  } else {
    userId = query.userId;
    if (!userId) {
      const user = await request(`users/@${encodeURIComponent(query.user)}/osu`);
      if (!Number.isSafeInteger(user.id) || user.id <= 0) throw new Error('官方玩家响应无效');
      userId = String(user.id);
    }
    if (query.selection === 'best') {
      score = (await request(`beatmaps/${query.bid}/scores/users/${userId}?mode=osu&legacy_only=0`)).score;
    } else {
      const candidates = [];
      // 仅扫描官方保留的近期记录，限制为 1000 条，不能当成无限历史查询。
      for (let offset = 0; offset < 1000; offset += 100) {
        const scores = await request(`users/${userId}/scores/recent?mode=osu&include_fails=1&legacy_only=0&limit=100&offset=${offset}`);
        if (!Array.isArray(scores)) throw new Error('官方近期成绩响应无效');
        for (const entry of scores) if (String(entry.beatmap_id) === query.bid) {
          const timestamp = Date.parse(entry.ended_at);
          if (!Number.isFinite(timestamp)) throw new Error('官方近期成绩时间无效');
          candidates.push({ entry, timestamp });
        }
        if (scores.length < 100) break;
      }
      candidates.sort((a, b) => b.timestamp - a.timestamp || b.entry.id - a.entry.id);
      score = candidates[0]?.entry;
      if (!score) throw new Error('近期最多 1000 条记录内没有该谱面的成绩；不会回退到最高分');
    }
    if (String(score?.user_id) !== userId || String(score?.beatmap_id) !== query.bid) throw new Error('官方返回的玩家或谱面 ID 不符');
  }
  if (!score || !Number.isSafeInteger(score.id) || score.id <= 0 || !Number.isSafeInteger(score.beatmap?.id) || score.beatmap.id <= 0 || score.beatmap_id !== score.beatmap.id || !Number.isSafeInteger(score.beatmap.beatmapset_id) || score.beatmap.beatmapset_id <= 0 || !/^[a-f0-9]{32}$/i.test(score.beatmap.checksum ?? '')) throw new Error('官方成绩响应缺少有效谱面信息');
  if (score.ruleset_id !== 0) throw new Error('只支持原生 Standard 成绩');
  if (!score.has_replay) throw new Error('选中的成绩没有可下载回放');
  return { scoreId: String(score.id), bid: String(score.beatmap.id), setId: String(score.beatmap.beatmapset_id), checksum: score.beatmap.checksum.toLowerCase() };
}
