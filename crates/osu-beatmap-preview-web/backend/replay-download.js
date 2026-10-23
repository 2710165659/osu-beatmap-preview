// 在线回放使用固定官方地址，限制体积并在写缓存前核对头部 MD5；完整规则验证由 WASM 完成。
import fsp from 'node:fs/promises';
import path from 'node:path';
import { writeFileAtomic } from './cache.js';
import { selectReplayScore } from './replay-scores.js';

const LIMIT = 64 * 1024 * 1024;
function verify(bytes, checksum) {
  // 头部包含模式、版本和 32 字节谱面 MD5；只允许当前支持的 Standard/Taiko 回放。
  if (!bytes.length || bytes.length > LIMIT || ![0, 1].includes(bytes[0]) || bytes[5] !== 0x0b || bytes[6] !== 32 || bytes.subarray(7, 39).toString('ascii').toLowerCase() !== checksum) throw new Error('BeatmapMismatch: 官方回放头部无效或谱面 MD5 不符');
}
export function createReplayDownload({ api, cacheDir, noCache = false }) {
  const pending = new Map();
  async function bytesFor(score) {
    const directory = path.join(cacheDir, 'osr-download-cache');
    const target = path.join(directory, `${score.scoreId}.osr`);
    if (!noCache) {
      try {
        if ((await fsp.stat(target)).size <= LIMIT) {
          const bytes = await fsp.readFile(target); verify(bytes, score.checksum); return bytes;
        }
      } catch { /* 缺失或坏缓存重新下载，错误响应不能成为可用回放。 */ }
    }
    const response = await api.response(`scores/${score.scoreId}/download`);
    if (Number(response.headers.get('content-length')) > LIMIT) throw new Error('回放超过 64 MiB');
    const chunks = [];
    let length = 0;
    try {
      for await (const chunk of response.body) {
        length += chunk.length;
        if (length > LIMIT) throw new Error('回放超过 64 MiB');
        chunks.push(chunk);
      }
    } catch (error) {
      throw new Error(error.message === '回放超过 64 MiB' ? error.message : '官方回放读取失败');
    }
    const bytes = Buffer.concat(chunks); verify(bytes, score.checksum);
    await fsp.mkdir(directory, { recursive: true });
    await writeFileAtomic(target, bytes);
    return bytes;
  }
  return async (query) => {
    const score = await selectReplayScore(api, query);
    // 同一成绩的并发下载共用一次文件请求，成绩列表不缓存，避免“最近”停在旧记录。
    const key = `${score.scoreId}:${score.checksum}`;
    if (!pending.has(key)) pending.set(key, bytesFor(score).finally(() => pending.delete(key)));
    return { score, bytes: await pending.get(key) };
  };
}
