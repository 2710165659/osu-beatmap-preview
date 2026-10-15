// 官方 checksum 查询；与在线回放共用服务端认证客户端。
import { createOfficialApi } from './official-api.js';
export function createReplayLookup(options = {}) {
  const api = options.api ?? createOfficialApi(options);
  return async function lookup(checksum) {
    if (!/^[a-f0-9]{32}$/i.test(checksum ?? '')) throw new Error('非法回放谱面 MD5');
    const result = await api.get(`beatmaps/lookup?checksum=${checksum.toLowerCase()}`);
    if (!Number.isSafeInteger(result.id) || result.id <= 0 || !Number.isSafeInteger(result.beatmapset_id) || result.beatmapset_id <= 0 || result.checksum?.toLowerCase() !== checksum.toLowerCase()) throw new Error('官方查询返回的谱面与回放 MD5 不符');
    return { bid: String(result.id), setId: String(result.beatmapset_id), checksum: result.checksum.toLowerCase() };
  };
}
