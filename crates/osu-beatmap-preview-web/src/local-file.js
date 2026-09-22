// 本地谱面文件（.osu / .osz）在浏览器里的读取规则。
//
// 与 CLI 的 `application/local.rs` 是同一套判定（`test/local-file.test.js` 用同一张
// 用例表钉住两边）：
// - 文件类型按扩展名（`.osu` / `.osz`，不区分大小写）；
// - `.osz` 里只认**顶层** `.osu`：stable 会忽略子目录里的谱面，osu!lazer 的
//   `BeatmapImporter` 同样按 `!f.Filename.Contains('/')` 过滤；
// - 多难度按 `.osu` 的 `[Metadata] BeatmapID` 匹配 bid，匹配不到直接报错。
//
// 另外提供两件本地预览需要的小工具：谱面自带打击音的条目筛选（限制与后端
// `resources.js` 一致），以及本地 `.osu` 没有音频时充当播放时钟的静音 WAV。

import { normalizeArchivePath } from './zip.js';

/** 谱面自带打击音允许的扩展名（与 core 的 `SAMPLE_EXTENSIONS` 一致）。 */
const SAMPLE_EXTENSIONS = new Set(['ogg', 'wav', 'mp3']);
/** 单个样本的大小上限（8 MiB）：超过它的一定不是打击音。 */
const MAX_SAMPLE_BYTES = 8 * 1024 * 1024;
/** 一次最多取用的样本条目数。 */
const MAX_SAMPLE_ENTRIES = 64;
/** 全部样本的合计上限（32 MiB）。 */
const MAX_SAMPLE_TOTAL_BYTES = 32 * 1024 * 1024;

/**
 * 本地输入文件类型（按扩展名，不区分大小写）。
 *
 * @param {string} name 文件名（可以带路径）
 * @returns {'osu' | 'osz' | null} 不支持的后缀返回 null
 */
export function localFileKind(name) {
  const text = String(name ?? '');
  const dot = text.lastIndexOf('.');
  const extension = dot >= 0 ? text.slice(dot + 1).toLowerCase() : '';
  if (extension === 'osu') return 'osu';
  if (extension === 'osz') return 'osz';
  return null;
}

/** 压缩包条目是否是可预览的 `.osu` 谱面文件（顶层、后缀 `.osu`）。 */
export function isBeatmapEntry(name) {
  const normalized = normalizeArchivePath(name);
  if (!normalized) return false;
  // stable 会忽略子目录里的 .osu，osu!lazer 同样只取顶层条目。
  if (normalized.includes('/')) return false;
  return normalized.toLowerCase().endsWith('.osu');
}

/**
 * 列出压缩包里的谱面条目（顶层 `.osu`），保持压缩包内顺序，结果稳定可复现。
 *
 * @param {{name: string}[]} entries `readZipIndex` 的结果
 */
export function beatmapEntries(entries) {
  return (entries ?? []).filter((entry) => isBeatmapEntry(entry.name));
}

/**
 * 谱面自带打击音条目筛选。
 *
 * 限制与后端 `resources.js` 的 `#extractSamples` 一致：只按扩展名筛「像打击音」的
 * 条目、跳过歌曲音频本身，最多 64 个 / 单个 8 MiB / 合计 32 MiB。返回 `[{ name, entry }]`，
 * 顺序即压缩包内顺序。
 *
 * @param {{name: string, uncompressedSize: number}[]} entries
 * @param {string | null} audioEntryName 歌曲音频条目名（跳过它）
 */
export function sampleEntries(entries, audioEntryName) {
  const audioName = String(audioEntryName ?? '').toLowerCase();
  const picked = [];
  let total = 0;
  for (const entry of entries ?? []) {
    if (picked.length >= MAX_SAMPLE_ENTRIES || total >= MAX_SAMPLE_TOTAL_BYTES) break;
    const name = normalizeArchivePath(entry.name);
    if (!name || !isSampleEntry(name) || name.toLowerCase() === audioName) continue;
    if (entry.uncompressedSize > MAX_SAMPLE_BYTES) continue;
    picked.push({ name, entry });
    total += entry.uncompressedSize;
  }
  return picked;
}

/**
 * 生成一段静音 WAV（PCM 16-bit 单声道 8 kHz）。
 *
 * 本地 `.osu` 没有音频文件，而播放时钟完全由 HTML 音频元素驱动（seek、倍速、
 * 暂停对齐都依赖它），所以用一段静音当时间轴：画面与打击音照常走同一条时钟，
 * 只是听不到音乐。8 kHz 单声道下每分钟约 0.9 MiB，内存代价可以忽略。
 *
 * @param {number} seconds 静音时长（秒）
 * @returns {Uint8Array} 可直接喂给 `new Blob([...])` 的 WAV 字节
 */
export function silentWavBytes(seconds) {
  const sampleRate = 8000;
  const samples = Math.max(1, Math.ceil((Number(seconds) || 0) * sampleRate));
  const bytes = new Uint8Array(44 + samples * 2);
  const view = new DataView(bytes.buffer);
  writeAscii(view, 0, 'RIFF');
  view.setUint32(4, 36 + samples * 2, true);
  writeAscii(view, 8, 'WAVE');
  writeAscii(view, 12, 'fmt ');
  view.setUint32(16, 16, true); // PCM 格式块长度
  view.setUint16(20, 1, true); // PCM 编码
  view.setUint16(22, 1, true); // 单声道
  view.setUint32(24, sampleRate, true);
  view.setUint32(28, sampleRate * 2, true); // 字节率（每样本 2 字节）
  view.setUint16(32, 2, true); // 块对齐
  view.setUint16(34, 16, true); // 位深
  writeAscii(view, 36, 'data');
  view.setUint32(40, samples * 2, true);
  // 样本区保持全零即为静音，无需再填。
  return bytes;
}

/** 条目是否是打击音候选（ogg / wav / mp3）。 */
function isSampleEntry(name) {
  const dot = name.lastIndexOf('.');
  if (dot < 0) return false;
  return SAMPLE_EXTENSIONS.has(name.slice(dot + 1).toLowerCase());
}

function writeAscii(view, offset, text) {
  for (let index = 0; index < text.length; index += 1) {
    view.setUint8(offset + index, text.charCodeAt(index));
  }
}
