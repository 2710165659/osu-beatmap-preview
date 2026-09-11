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
import { mimeFor, REQUEST_DEADLINE } from './config.js';
import { extractEntry, findEntry, normalizeArchivePath, readZipIndex } from './zip.js';

export class ResourceProvider {
  constructor({ cache, log }) {
    this.cache = cache;
    this.log = log;
    this.osuInflight = new Map();
    this.mediaInflight = new Map();
  }

  /** 下载（或命中缓存）`.osu`，返回本地路径。 */
  async beatmapPath(bid, { fresh = false } = {}) {
    return this.#singleFlight(this.osuInflight, bid, () =>
      downloadBeatmapFile({ bid, cache: this.cache, log: this.log, fresh }),
    );
  }

  /** 返回该 bid 的音频与背景文件信息。 */
  async media(bid, { fresh = false } = {}) {
    return this.#singleFlight(this.mediaInflight, bid, () => this.#prepareMedia(bid, fresh));
  }

  #singleFlight(registry, key, task) {
    const existing = registry.get(key);
    if (existing) return existing;
    const promise = task().finally(() => registry.delete(key));
    registry.set(key, promise);
    return promise;
  }

  async #prepareMedia(bid, fresh) {
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
      return {
        audio: { ...audio, mime: mimeFor(audio.path) },
        background: { ...background, mime: mimeFor(background.path) },
      };
    }

    let oszPath = await downloadBeatmapsetArchive({
      cache: this.cache,
      setId,
      bid,
      log: this.log,
      deadlineAt: Date.now() + REQUEST_DEADLINE,
      fresh,
    });
    try {
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
      });
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
