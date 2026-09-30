// 浏览器需要的资源：一份完整的谱面包（`.osz`）。
//
// 浏览器无法直接跨域下载 osu! 的资源，所以这里由 Node 后端只做下载与缓存：
//   1. 下载并缓存 `.osu`（拿到谱面包的 set id）与 `.osz`（多镜像竞速，见 download-osz.js）；
//   2. 把整包字节原样发给浏览器——`.osz` 的解包、音乐/背景/音效的解码与混音全部在
//      WASM 内完成，后端不看压缩包内部；
//   3. 对同一 bid 的并发请求做单飞，避免重复下载。

import { parseBeatmapText } from './beatmap-meta.js';
import { downloadBeatmapFile, readBeatmapText, resolveBeatmapSetId } from './download-osu.js';
import { downloadBeatmapsetArchive } from './download-osz.js';
import { PROGRESS_TTL, REQUEST_DEADLINE } from './config.js';

export class ResourceProvider {
  constructor({ cache, log }) {
    this.cache = cache;
    this.log = log;
    this.osuInflight = new Map();
    this.fileInflight = new Map();
    this.progressEntries = new Map();
  }

  /** 下载（或命中缓存）`.osu`，返回本地路径。 */
  async beatmapPath(bid, { fresh = false } = {}) {
    this.#report(bid, { phase: 'osu', received: 0, total: 0, message: '定位谱面' });
    return this.#singleFlight(this.osuInflight, bid, () =>
      downloadBeatmapFile({ bid, cache: this.cache, log: this.log, fresh }),
    );
  }

  /**
   * 下载（或命中缓存）该 bid 的完整谱面包（`.osz`），返回本地路径。
   *
   * 一切媒体资源都在这个包里：前端把它整份交给 WASM，解包与解码都在那边完成。
   */
  async file(bid, { fresh = false } = {}) {
    try {
      const oszPath = await this.#singleFlight(this.fileInflight, bid, () => this.#prepareFile(bid, fresh));
      this.#report(bid, { phase: 'ready', message: '' });
      return oszPath;
    } catch (error) {
      this.#report(bid, { phase: 'error', message: error.message });
      throw error;
    }
  }

  /**
   * 客户端正在读取这份资源。
   *
   * `transfer` 阶段描述的是「服务端把字节发给浏览器」这段时间，浏览器自己的接收进度
   * 无法从这里观测，只能由前端按 Content-Length 统计；这里负责把阶段推进过去，
   * 否则前端会一直停在服务端阶段，云端部署时看起来就像卡住了。
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

  async #prepareFile(bid, fresh) {
    // `.osu` 只用来拿 set id（下载 URL 的重定向里也有，但缓存命中时不必再多一次网络）。
    this.#report(bid, { phase: 'osu', received: 0, total: 0, message: '定位谱面' });
    const beatmapPath = await this.beatmapPath(bid, { fresh });
    const beatmap = parseBeatmapText(await readBeatmapText(beatmapPath));
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
    return downloadBeatmapsetArchive({
      cache: this.cache,
      setId,
      bid,
      log: this.log,
      deadlineAt: Date.now() + REQUEST_DEADLINE,
      fresh,
      onProgress,
    });
  }
}
