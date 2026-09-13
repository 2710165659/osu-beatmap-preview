// 浏览器需要的三类资源：谱面文本、音频、背景图。
//
// 浏览器无法直接跨域下载 osu! 的资源，所以这里由 Node 后端负责：
//   1. 下载并缓存 `.osu` 与 `.osz`（多镜像竞速，见 download-osz.js）；
//   2. 从 OSZ 里取出 `.osu` 声明的音频与背景，缓存到 media/<bid>/；
//   3. 对同一 bid 的并发请求做单飞，避免背景与音频两个请求各下载一次 OSZ。

import fsp from 'node:fs/promises';
import path from 'node:path';
import { existsNonEmpty, writeFileAtomic } from './cache.js';
import { parseBeatmapText } from './beatmap-meta.js';
import { downloadBeatmapFile, readBeatmapText, resolveBeatmapSetId } from './download-osu.js';
import { downloadBeatmapsetArchive } from './download-osz.js';
import { mimeFor, PROGRESS_TTL, REQUEST_DEADLINE } from './config.js';
import { extractEntry, findEntry, normalizeArchivePath, readZipIndex } from './zip.js';

export class ResourceProvider {
  constructor({ cache, log }) {
    this.cache = cache;
    this.log = log;
    this.osuInflight = new Map();
    this.mediaInflight = new Map();
    this.progressEntries = new Map();
  }

  /** 下载（或命中缓存）`.osu`，返回本地路径。 */
  async beatmapPath(bid, { fresh = false } = {}) {
    this.#report(bid, { phase: 'osu', received: 0, total: 0, message: '获取谱面' });
    return this.#singleFlight(this.osuInflight, bid, () =>
      downloadBeatmapFile({ bid, cache: this.cache, log: this.log, fresh }),
    );
  }

  /** 返回该 bid 的音频与背景文件信息。 */
  async media(bid, { fresh = false } = {}) {
    try {
      const media = await this.#singleFlight(this.mediaInflight, bid, () => this.#prepareMedia(bid, fresh));
      this.#report(bid, { phase: 'ready', message: '' });
      return media;
    } catch (error) {
      this.#report(bid, { phase: 'error', message: error.message });
      throw error;
    }
  }

  /**
   * 客户端正在读取这份资源。
   *
   * `media` 阶段描述的是「服务端把字节发给浏览器」这段时间，浏览器自己的接收进度
   * 无法从这里观测，只能由前端按 Content-Length 统计；这里负责把阶段推进过去，
   * 否则前端会一直停在「解析资源」，云端部署时看起来就像卡住了。
   */
  reportTransfer(bid, { total = 0, message = '传输到客户端' } = {}) {
    this.#report(bid, { phase: 'transfer', received: 0, total, message });
  }

  /** 加载进度快照，供 `/resource/progress` 轮询。 */
  progress(bid) {
    return this.progressEntries.get(bid) ?? { phase: 'idle', received: 0, total: 0, message: '' };
  }

  /** 记录一次进度；条目按 PROGRESS_TTL 自动淘汰，避免长时间运行后越攒越多。 */
  #report(bid, patch) {
    this.progressEntries.set(bid, { ...this.progressEntries.get(bid), ...patch, at: Date.now() });
    const deadline = Date.now() - PROGRESS_TTL;
    for (const [key, entry] of this.progressEntries) {
      if (entry.at < deadline) this.progressEntries.delete(key);
    }
  }

  #singleFlight(registry, key, task) {
    const existing = registry.get(key);
    if (existing) return existing;
    const promise = task().finally(() => registry.delete(key));
    registry.set(key, promise);
    return promise;
  }

  async #prepareMedia(bid, fresh) {
    this.#report(bid, { phase: 'osu', received: 0, total: 0, message: '获取谱面' });
    const beatmapPath = await this.beatmapPath(bid, { fresh });
    const beatmap = parseBeatmapText(await readBeatmapText(beatmapPath));
    if (!beatmap.audioFilename) {
      throw new Error('谱面未指定音频文件');
    }
    if (!beatmap.backgroundFilename) {
      throw new Error('谱面未指定背景文件');
    }
    const setId =
      beatmap.beatmapSetId ??
      (await resolveBeatmapSetId({ bid, log: this.log, signal: undefined, deadlineAt: Date.now() + REQUEST_DEADLINE }));

    const audio = mediaTarget(this.cache, bid, 'audio', beatmap.audioFilename);
    const background = mediaTarget(this.cache, bid, 'background', beatmap.backgroundFilename);
    if (!fresh && existsNonEmpty(audio.path) && existsNonEmpty(background.path)) {
      this.#report(bid, { phase: 'ready', received: 0, total: 0, message: '' });
      return {
        audio: { ...audio, mime: mimeFor(audio.path) },
        background: { ...background, mime: mimeFor(background.path) },
      };
    }

    // OSZ 动辄几十 MiB，是加载里最慢的一步：把字节数、镜像名透出去给前端的进度条。
    const onProgress = ({ received, total, mirror, connected }) => {
      const stage = connected ? '服务端下载谱面包' : '服务端连接镜像';
      this.#report(bid, {
        phase: 'osz',
        received,
        total,
        message: mirror ? `${stage} · ${mirror}` : stage,
      });
    };
    this.#report(bid, { phase: 'osz', received: 0, total: 0, message: '服务端连接镜像' });

    let oszPath = await downloadBeatmapsetArchive({
      cache: this.cache,
      setId,
      bid,
      log: this.log,
      deadlineAt: Date.now() + REQUEST_DEADLINE,
      fresh,
      onProgress,
    });
    try {
      this.#report(bid, { phase: 'extract', received: 0, total: 0, message: '服务端解包音频与背景' });
      await this.#extractMedia(oszPath, audio, background);
    } catch (error) {
      // 缓存里的压缩包可能损坏：丢掉后重新下载一次再试。
      this.log(`OSZ 解包失败，重新下载：${error.message}`);
      await fsp.rm(oszPath, { force: true });
      oszPath = await downloadBeatmapsetArchive({
        cache: this.cache,
        setId,
        bid,
        log: this.log,
        deadlineAt: Date.now() + REQUEST_DEADLINE,
        fresh: true,
        onProgress,
      });
      this.#report(bid, { phase: 'extract', received: 0, total: 0, message: '服务端解包音频与背景' });
      await this.#extractMedia(oszPath, audio, background);
    }
    return {
      audio: { ...audio, mime: mimeFor(audio.path) },
      background: { ...background, mime: mimeFor(background.path) },
    };
  }

  async #extractMedia(oszPath, audio, background) {
    const buffer = await fsp.readFile(oszPath);
    const entries = readZipIndex(buffer);
    const audioEntry = findEntry(entries, audio.entryName);
    if (!audioEntry) throw new Error(`OSZ 中找不到资源：${audio.entryName}`);
    const backgroundEntry = findEntry(entries, background.entryName);
    if (!backgroundEntry) throw new Error(`OSZ 中找不到资源：${background.entryName}`);
    await writeFileAtomic(audio.path, extractEntry(buffer, audioEntry));
    await writeFileAtomic(background.path, extractEntry(buffer, backgroundEntry));
  }
}

function mediaTarget(cache, bid, stem, entryName) {
  const normalized = normalizeArchivePath(entryName);
  if (!normalized) {
    throw new Error(`压缩包资源路径非法：${entryName}`);
  }
  const extension = path.extname(normalized).replace(/^\./, '').toLowerCase() || 'bin';
  return {
    entryName: normalized,
    path: cache.mediaPath(bid, stem, extension),
  };
}
