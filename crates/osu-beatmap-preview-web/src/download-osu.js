// `.osu` 文件下载与谱面集 ID 解析。
//
// 对应 CLI 的 download::osu：先查缓存，再用 osu.ppy.sh 的接口下载；`.osu` 里缺少
// BeatmapSetID 时，跟随 https://osu.ppy.sh/beatmaps/<bid> 的重定向拿到真实 set id。

import fsp from 'node:fs/promises';
import { OSU_REQUEST_TIMEOUT, USER_AGENT } from './config.js';
import { existsNonEmpty, writeFileAtomic } from './cache.js';
import { abortError, drain, httpRequest, isAbortError } from './http.js';

/** `.osu` 文件的大小上限，避免异常响应把内存撑满。 */
const MAX_OSU_BYTES = 16 * 1024 * 1024;

export async function downloadBeatmapFile({ bid, cache, log, signal, fresh = false }) {
  const target = cache.osuPath(bid);
  if (!fresh && existsNonEmpty(target)) {
    log(`.osu 命中缓存：${bid}.osu`);
    return target;
  }

  const url = `https://osu.ppy.sh/osu/${bid}`;
  const response = await httpRequest(url, {
    headers: { 'User-Agent': USER_AGENT },
    signal,
    idleTimeoutMs: OSU_REQUEST_TIMEOUT,
    connectTimeoutMs: OSU_REQUEST_TIMEOUT,
  });
  if (response.status === 404) {
    await drain(response.body);
    throw new Error(`找不到谱面 ${bid}`);
  }
  if (response.status !== 200) {
    await drain(response.body);
    throw new Error(`下载谱面 ${bid} 失败：HTTP ${response.status}`);
  }

  const chunks = [];
  let total = 0;
  try {
    for await (const chunk of response.body) {
      if (signal?.aborted) throw abortError();
      total += chunk.length;
      if (total > MAX_OSU_BYTES) throw new Error(`谱面 ${bid} 超过大小上限`);
      chunks.push(chunk);
    }
  } catch (error) {
    if (isAbortError(error)) throw error;
    throw new Error(`下载谱面 ${bid} 失败：${error.message}`);
  }
  if (total === 0) throw new Error(`谱面 ${bid} 的响应为空`);

  await writeFileAtomic(target, Buffer.concat(chunks, total));
  log(`.osu 下载完成：${bid}.osu（${(total / 1024).toFixed(1)} KiB）`);
  return target;
}

/** 跟随 osu.ppy.sh 的重定向解析真实谱面集 ID。 */
export async function resolveBeatmapSetId({ bid, log, signal }) {
  const url = `https://osu.ppy.sh/beatmaps/${bid}`;
  const response = await httpRequest(url, {
    method: 'HEAD',
    headers: { 'User-Agent': USER_AGENT },
    signal,
    idleTimeoutMs: OSU_REQUEST_TIMEOUT,
    connectTimeoutMs: OSU_REQUEST_TIMEOUT,
  });
  await drain(response.body);
  const setId = beatmapSetIdFromUrl(response.url);
  if (!setId) {
    throw new Error(`无法解析谱面 ${bid} 的谱面集 ID（重定向到 ${response.url}）`);
  }
  log(`谱面集 ID 解析结果：bid=${bid} set=${setId}`);
  return setId;
}

export function beatmapSetIdFromUrl(url) {
  const match = /^https:\/\/osu\.ppy\.sh\/beatmapsets\/(\d+)/.exec(String(url));
  if (!match) return null;
  const setId = Number(match[1]);
  return Number.isInteger(setId) && setId > 0 ? setId : null;
}

/** 读取缓存的 `.osu` 文本。 */
export async function readBeatmapText(path) {
  return fsp.readFile(path, 'utf8');
}
