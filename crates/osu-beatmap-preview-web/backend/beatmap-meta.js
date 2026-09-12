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
  return {
    sections,
    general,
    metadata,
    audioFilename: nonEmpty(general.get('AudioFilename')),
    beatmapSetId: parseSetId(metadata.get('BeatmapSetID')),
    backgroundFilename: parseBackgroundFilename(sections.get('Events')),
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

function splitFirst(text, separator, limit) {
  const parts = text.split(separator);
  if (parts.length <= limit) return parts;
  return [...parts.slice(0, limit - 1), parts.slice(limit - 1).join(separator)];
}
