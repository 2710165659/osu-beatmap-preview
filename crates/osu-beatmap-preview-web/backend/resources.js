// 浏览器需要的三类资源：谱面文本、音频、背景图，外加谱面自带的打击音样本。
//
// 浏览器无法直接跨域下载 osu! 的资源，所以这里由 Node 后端负责：
//   1. 下载并缓存 `.osu` 与 `.osz`（多镜像竞速，见 download-osz.js）；
//   2. 从 OSZ 里取出 `.osu` 声明的音频与背景，缓存到 media/<bid>/；
//   3. 顺带解出谱面自带的打击音样本（同类音频文件），前端按需取用；
//   4. 对同一 bid 的并发请求做单飞，避免背景与音频两个请求各下载一次 OSZ。

import fsp from 'node:fs/promises';
import path from 'node:path';
import { existsNonEmpty, writeFileAtomic } from './cache.js';
import { parseBeatmapText } from './beatmap-meta.js';
import { downloadBeatmapFile, readBeatmapText, resolveBeatmapSetId } from './download-osu.js';
import { downloadBeatmapsetArchive } from './download-osz.js';
import { mimeFor, PROGRESS_TTL, REQUEST_DEADLINE } from './config.js';
import { extractEntry, findEntry, normalizeArchivePath, readZipIndex } from './zip.js';

/**
 * 谱面自带打击音允许的扩展名。
 *
 * 与 core 的 `SAMPLE_EXTENSIONS`（`domain/media.rs`）一致：多解一个文件只是浪费磁盘，
 * 少解一个则会让某个音效退回内嵌皮肤，因此两边必须同步。
 */
const SAMPLE_EXTENSIONS = new Set(['ogg', 'wav', 'mp3']);
/** 单个样本的大小上限（8 MiB）：超过它的一定不是打击音。 */
const MAX_SAMPLE_BYTES = 8 * 1024 * 1024;
/** 一次最多解出的样本条目数。 */
const MAX_SAMPLE_ENTRIES = 64;
/** 全部样本解出后的合计上限（32 MiB）。 */
const MAX_SAMPLE_TOTAL_BYTES = 32 * 1024 * 1024;
/** 样本清单的缓存文件名（`media/<bid>/samples.json`）。 */
const SAMPLE_MANIFEST = 'samples';

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

  /** 返回该 bid 的音频、背景文件信息与谱面自带打击音清单。 */
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
   * 取出一个谱面自带打击音样本的缓存路径与 MIME。
   *
   * 只在清单里按条目名查找（不区分大小写），因此不会因为前端传入任意路径而读到
   * 缓存目录之外的文件；清单里没有这个条目时返回 `null`。
   */
  async sample(bid, name) {
    const media = await this.media(bid, { fresh: false });
    const wanted = String(name ?? '').toLowerCase();
    const target = (media.samples ?? []).find((entry) => entry.name.toLowerCase() === wanted);
    if (!target) return null;
    return { ...target, mime: mimeFor(target.path) };
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
    // 音频必需；背景可选：没有声明（或路径非法）时前端会退化成纯色背景，
    // 不能因为缺背景把整张谱面（连同音乐）一起判为加载失败。
    const audio = mediaTarget(this.cache, bid, 'audio', beatmap.audioFilename);
    if (!audio) {
      throw new Error(`压缩包资源路径非法：${beatmap.audioFilename}`);
    }
    const background = mediaTarget(this.cache, bid, 'background', beatmap.backgroundFilename);
    if (!fresh && existsNonEmpty(audio.path) && (!background || existsNonEmpty(background.path))) {
      this.#report(bid, { phase: 'ready', received: 0, total: 0, message: '' });
      return {
        audio: { ...audio, mime: mimeFor(audio.path) },
        background: background ? { ...background, mime: mimeFor(background.path) } : null,
        samples: await this.#cachedSamples(bid, beatmap.beatmapSetId, audio.entryName),
      };
    }

    const setId =
      beatmap.beatmapSetId ??
      (await resolveBeatmapSetId({ bid, log: this.log, signal: undefined, deadlineAt: Date.now() + REQUEST_DEADLINE }));

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
    // 解包会回报「实际可用的背景」：声明了但压缩包里没有时退化成 null。
    let resolvedBackground = background;
    let samples = [];
    try {
      this.#report(bid, { phase: 'extract', received: 0, total: 0, message: '服务端解包音频与音效' });
      ({ background: resolvedBackground, samples } = await this.#extractMedia(oszPath, audio, background, bid));
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
      this.#report(bid, { phase: 'extract', received: 0, total: 0, message: '服务端解包音频与音效' });
      ({ background: resolvedBackground, samples } = await this.#extractMedia(oszPath, audio, background, bid));
    }
    return {
      audio: { ...audio, mime: mimeFor(audio.path) },
      background: resolvedBackground
        ? { ...resolvedBackground, mime: mimeFor(resolvedBackground.path) }
        : null,
      samples,
    };
  }

  /**
   * 从 OSZ 里取出音频（必需）、背景（可选）与谱面自带的打击音样本。
   *
   * 返回实际可用的背景条目与样本清单：声明了背景但压缩包里找不到时返回 `null`（只影响
   * 背景），音频条目缺失则直接抛错——没有音乐时预览没有意义。样本是可选增强，
   * 任何单个条目解不出来都只影响那一个音效（前端会退回内嵌皮肤）。
   */
  async #extractMedia(oszPath, audio, background, bid) {
    const buffer = await fsp.readFile(oszPath);
    const entries = readZipIndex(buffer);
    const audioEntry = findEntry(entries, audio.entryName);
    if (!audioEntry) throw new Error(`OSZ 中找不到资源：${audio.entryName}`);
    await writeFileAtomic(audio.path, extractEntry(buffer, audioEntry));
    const samples = await this.#extractSamples(buffer, entries, bid, audio.entryName);
    if (!background) return { background: null, samples };
    const backgroundEntry = findEntry(entries, background.entryName);
    if (!backgroundEntry) {
      this.log(`OSZ 中找不到背景：${background.entryName}，将使用纯色背景`);
      return { background: null, samples };
    }
    await writeFileAtomic(background.path, extractEntry(buffer, backgroundEntry));
    return { background, samples };
  }

  /**
   * 解出谱面自带的打击音样本，并写下清单。
   *
   * 只按扩展名筛「像打击音」的条目，不猜候选名：真正要播哪几个由前端的 wasm 决定
   * （`hitsoundRequiredNames`），后端多解出来的文件不会再被请求，代价只是磁盘。
   * 这样候选名的规则仍然只有 core 一份，Node 侧不需要再实现一遍。
   * 歌曲音频（`audioEntryName`）本身不是打击音，直接跳过。
   */
  async #extractSamples(buffer, entries, bid, audioEntryName) {
    const written = [];
    const audioName = String(audioEntryName ?? '').toLowerCase();
    let total = 0;
    for (const entry of entries) {
      if (written.length >= MAX_SAMPLE_ENTRIES || total >= MAX_SAMPLE_TOTAL_BYTES) break;
      const name = normalizeArchivePath(entry.name);
      if (!name || !isSampleEntry(name) || name.toLowerCase() === audioName) continue;
      if (entry.uncompressedSize > MAX_SAMPLE_BYTES) continue;
      const target = sampleTarget(this.cache, bid, name);
      if (!target) continue;
      try {
        await writeFileAtomic(target.path, extractEntry(buffer, entry));
      } catch (error) {
        // 单个条目压缩方式异常/损坏时只跳过它：不能因为一个音效让整张谱面加载失败。
        this.log(`跳过无法解出的谱面音效 ${name}：${error.message}`);
        continue;
      }
      written.push(name);
      total += entry.uncompressedSize;
    }
    await writeFileAtomic(
      this.cache.mediaPath(bid, SAMPLE_MANIFEST, 'json'),
      JSON.stringify({ samples: written }),
    );
    return written.map((name) => sampleTarget(this.cache, bid, name)).filter(Boolean);
  }

  /**
   * 读取缓存里的样本清单；媒体缓存命中时走这里。
   *
   * 旧版本解包时没有写过清单：压缩包还在本地缓存里就补解一次并补写清单，否则老缓存会
   * 一直「没有谱面自带音效」，用户只能靠清缓存才能恢复。清单存在但为空（谱面本来就没有
   * 自定义音效）时不再重复解包。
   */
  async #cachedSamples(bid, setId, audioEntryName) {
    const manifest = await this.#readSampleManifest(bid);
    if (manifest !== null) return manifest;
    if (!setId) return [];
    const oszPath = this.cache.oszPath(setId);
    if (!existsNonEmpty(oszPath)) return [];
    try {
      return await this.#extractSamplesFromArchive(oszPath, bid, audioEntryName);
    } catch (error) {
      // 压缩包损坏/被删时只是没有谱面音效，音频与背景仍在缓存里，不影响预览。
      this.log(`补解谱面音效失败，将只用内嵌音效：${error.message}`);
      return [];
    }
  }

  /**
   * 读取样本清单；文件缺失或损坏时返回 `null`（调用方据此决定要不要补解）。
   */
  async #readSampleManifest(bid) {
    let payload;
    try {
      payload = JSON.parse(await fsp.readFile(this.cache.mediaPath(bid, SAMPLE_MANIFEST, 'json'), 'utf8'));
    } catch {
      return null;
    }
    const names = Array.isArray(payload?.samples) ? payload.samples : [];
    return names
      .filter((name) => typeof name === 'string')
      .map((name) => sampleTarget(this.cache, bid, name))
      .filter(Boolean);
  }

  /** 打开压缩包、解出全部谱面音效并写下清单。 */
  async #extractSamplesFromArchive(oszPath, bid, audioEntryName) {
    const buffer = await fsp.readFile(oszPath);
    return this.#extractSamples(buffer, readZipIndex(buffer), bid, audioEntryName);
  }
}

/**
 * 谱面样本的缓存条目。
 *
 * `name` 是压缩包里的条目名，前端据此匹配候选名并通过 `/resource/sample?name=` 取用；
 * `path` 是解包后的本地文件。条目名非法（越界路径）时返回 `null`。
 */
function sampleTarget(cache, bid, entryName) {
  const target = mediaTarget(cache, bid, sampleStem(entryName), entryName);
  return target ? { name: entryName, path: target.path } : null;
}

/** 条目名是否是打击音候选（ogg / wav / mp3）。 */
function isSampleEntry(name) {
  const extension = path.extname(name).replace(/^\./, '').toLowerCase();
  return SAMPLE_EXTENSIONS.has(extension);
}

/**
 * 样本的缓存文件名主干。
 *
 * 条目名可能带子目录、大小写也不固定，直接当文件名不安全；用 FNV-1a 32 位压成
 * 稳定且不重名的短名字（同一 bid 内不同条目撞哈希的概率可以忽略）。
 */
function sampleStem(entryName) {
  let hash = 0x811c9dc5;
  for (let index = 0; index < entryName.length; index += 1) {
    hash ^= entryName.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return `sample-${hash.toString(16).padStart(8, '0')}`;
}

/**
 * 计算某个媒体条目的缓存路径。
 *
 * 返回 `null` 表示入口不可用（文件名缺失或路径非法）：音频入口缺失是致命错误，
 * 背景入口缺失只意味着退化成纯色背景，由调用方决定怎么处理。
 */
function mediaTarget(cache, bid, stem, entryName) {
  const normalized = normalizeArchivePath(entryName);
  if (!normalized) {
    return null;
  }
  const extension = path.extname(normalized).replace(/^\./, '').toLowerCase() || 'bin';
  return {
    entryName: normalized,
    path: cache.mediaPath(bid, stem, extension),
  };
}
