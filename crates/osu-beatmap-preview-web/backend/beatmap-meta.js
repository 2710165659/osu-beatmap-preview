// `.osu` 文件里下载所需的少量字段解析。
//
// 只需要 [General] 的 AudioFilename、[Metadata] 的 BeatmapSetID 和 [Events] 的
// 背景图文件名。解析规则与 core 保持一致：区段按 [Name] 切分、键值按第一个冒号
// 切分、背景名支持引号包裹与反斜杠路径。

/** 按区段拆出 `.osu` 文本，返回各区的行数组。 */
export function splitSections(content) {
  const sections = new Map();
  let current = null;
  for (const raw of content.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith('//')) continue;
    if (line.startsWith('[') && line.endsWith(']')) {
      current = line.slice(1, -1);
      if (!sections.has(current)) sections.set(current, []);
      continue;
    }
    if (current) sections.get(current).push(line);
  }
  return sections;
}

/** 解析一次 `.osu`，返回后续步骤需要的字段。 */
export function parseBeatmapText(content) {
  const sections = splitSections(content);
  const general = parseKeyValue(sections.get('General'));
  const metadata = parseKeyValue(sections.get('Metadata'));
  const videoEvent = parseVideoEvent(sections.get('Events'));
  // 与 core 的解析规则一致：`Video,` 行指向非视频文件时按背景图处理（老谱面兼容），
  // 已有正经背景事件时不覆盖它；扩展名不在白名单内的行不算背景视频。
  const backgroundFilename = parseBackgroundFilename(sections.get('Events'))
    ?? (videoEvent && !isVideoEntry(videoEvent.filename) ? videoEvent.filename : null);
  const video = videoEvent && isVideoEntry(videoEvent.filename) ? videoEvent : null;
  return {
    sections,
    general,
    metadata,
    audioFilename: nonEmpty(general.get('AudioFilename')),
    beatmapSetId: parseSetId(metadata.get('BeatmapSetID')),
    backgroundFilename,
    videoFilename: video?.filename ?? null,
    videoStartMs: video?.startMs ?? null,
  };
}

function parseKeyValue(lines) {
  const values = new Map();
  for (const line of lines ?? []) {
    const separator = line.indexOf(':');
    if (separator < 0) continue;
    values.set(line.slice(0, separator).trim(), line.slice(separator + 1).trim());
  }
  return values;
}

function parseSetId(value) {
  const parsed = Number(value);
  return Number.isInteger(parsed) && parsed > 0 ? parsed : null;
}

function nonEmpty(value) {
  const text = String(value ?? '').trim();
  return text ? text : null;
}

/** 取 [Events] 里第一条背景声明（`0,0,"bg.jpg",0,0`）的文件名。 */
export function parseBackgroundFilename(lines) {
  for (const line of lines ?? []) {
    const fields = splitFirst(line, ',', 3);
    if (fields.length < 3 || fields[0].trim() !== '0') continue;
    const remainder = fields[2].trim();
    if (!remainder) continue;
    const name = remainder.startsWith('"')
      ? quoteContent(remainder)
      : remainder.split(',')[0].trim();
    if (name) return name.replaceAll('\\', '/');
  }
  return null;
}

function quoteContent(text) {
  const end = text.indexOf('"', 1);
  return end < 0 ? null : text.slice(1, end).trim();
}

/** osu! 的视频扩展名白名单；与 core `media::VIDEO_EXTENSIONS` 一致。 */
const VIDEO_EXTENSIONS = ['mp4', 'mov', 'avi', 'flv', 'mpg', 'wmv', 'm4v'];

/** 条目是否算背景视频（按扩展名，大小写不敏感）；与 core `is_video_entry` 一致。 */
export function isVideoEntry(name) {
  const file = String(name ?? '').replaceAll('\\', '/');
  const stem = file.slice(file.lastIndexOf('/') + 1);
  const dot = stem.lastIndexOf('.');
  if (dot < 0) return false;
  return VIDEO_EXTENSIONS.includes(stem.slice(dot + 1).toLowerCase());
}

/**
 * 取 [Events] 里第一条视频声明（`Video,100,"intro.mp4"` 或旧式 `1,...`）。
 *
 * 返回 `{ filename, startMs }`；起始时间允许写成小数（向零截断，写坏按 0），
 * 与 core 的 `parse_media_event` 同规则。**不做**扩展名过滤（调用方按需筛选）。
 */
export function parseVideoEvent(lines) {
  for (const line of lines ?? []) {
    for (const type of ['Video', '1']) {
      const fields = splitFirst(line, ',', 3);
      if (fields.length < 3 || fields[0].trim() !== type) continue;
      const remainder = fields[2].trim();
      if (!remainder) continue;
      const name = remainder.startsWith('"')
        ? quoteContent(remainder)
        : remainder.split(',')[0].trim();
      if (!name) continue;
      return { filename: name.replaceAll('\\', '/'), startMs: parseStartMs(fields[1].trim()) };
    }
  }
  return null;
}

function parseStartMs(value) {
  const parsed = Number(value);
  return Number.isFinite(parsed) ? Math.trunc(parsed) : 0;
}

function splitFirst(text, separator, limit) {
  const parts = text.split(separator);
  if (parts.length <= limit) return parts;
  return [...parts.slice(0, limit - 1), parts.slice(limit - 1).join(separator)];
}
