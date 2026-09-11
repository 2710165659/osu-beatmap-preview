// 最小 ZIP 读取实现。
//
// OSZ 就是 ZIP：这里只做「读中央目录 + 解出需要的条目」，不写压缩、不处理加密。
// 用 Node 自带的 zlib 解 deflate，因此后端不需要任何第三方依赖。

import zlib from 'node:zlib';

const EOCD_SIGNATURE = 0x06054b50;
const ZIP64_EOCD_SIGNATURE = 0x06064b50;
const ZIP64_LOCATOR_SIGNATURE = 0x07064b50;
const CENTRAL_SIGNATURE = 0x02014b50;
const LOCAL_SIGNATURE = 0x04034b50;
const EOCD_MIN_SIZE = 22;
const MAX_COMMENT_SIZE = 0xffff;
const ZIP64_EXTRA_ID = 0x0001;

/** 尾部是否包含 ZIP 结束记录；用于廉价地校验缓存文件。 */
export function hasZipEndRecord(buffer) {
  return findEndOfCentralDirectory(buffer) >= 0;
}

/** 完整解析中央目录，成功则说明是有效的 ZIP。 */
export function isZipBuffer(buffer) {
  try {
    readZipIndex(buffer);
    return true;
  } catch {
    return false;
  }
}

/**
 * 解析中央目录，返回条目名与条目信息。
 *
 * 条目名统一转成小写做键，便于与 `.osu` 里声明的文件名做大小写不敏感匹配。
 */
export function readZipIndex(buffer) {
  const eocd = findEndOfCentralDirectory(buffer);
  if (eocd < 0) throw new Error('缺少 ZIP 中央目录结束记录');

  let entryCount = buffer.readUInt16LE(eocd + 10);
  let directorySize = buffer.readUInt32LE(eocd + 12);
  let directoryOffset = buffer.readUInt32LE(eocd + 16);

  const zip64 = readZip64EndOfCentralDirectory(buffer, eocd);
  if (zip64) {
    entryCount = zip64.entryCount;
    directorySize = zip64.directorySize;
    directoryOffset = zip64.directoryOffset;
  }

  if (directoryOffset + directorySize > buffer.length) {
    throw new Error('ZIP 中央目录超出文件范围');
  }

  const entries = [];
  let cursor = directoryOffset;
  for (let index = 0; index < entryCount; index += 1) {
    if (cursor + 46 > buffer.length || buffer.readUInt32LE(cursor) !== CENTRAL_SIGNATURE) {
      throw new Error(`第 ${index} 个中央目录条目无效`);
    }
    const flags = buffer.readUInt16LE(cursor + 8);
    const method = buffer.readUInt16LE(cursor + 10);
    let compressedSize = buffer.readUInt32LE(cursor + 20);
    let uncompressedSize = buffer.readUInt32LE(cursor + 24);
    const nameLength = buffer.readUInt16LE(cursor + 28);
    const extraLength = buffer.readUInt16LE(cursor + 30);
    const commentLength = buffer.readUInt16LE(cursor + 32);
    let localHeaderOffset = buffer.readUInt32LE(cursor + 42);
    const nameStart = cursor + 46;
    const extraStart = nameStart + nameLength;
    if (extraStart + extraLength + commentLength > buffer.length) {
      throw new Error(`第 ${index} 个中央目录条目超出文件范围`);
    }
    const name = buffer.toString('utf8', nameStart, extraStart);

    if (
      compressedSize === 0xffffffff ||
      uncompressedSize === 0xffffffff ||
      localHeaderOffset === 0xffffffff
    ) {
      const zip64Values = readZip64Extra(
        buffer.subarray(extraStart, extraStart + extraLength),
        uncompressedSize === 0xffffffff,
        compressedSize === 0xffffffff,
        localHeaderOffset === 0xffffffff,
      );
      uncompressedSize = zip64Values.uncompressedSize ?? uncompressedSize;
      compressedSize = zip64Values.compressedSize ?? compressedSize;
      localHeaderOffset = zip64Values.localHeaderOffset ?? localHeaderOffset;
    }

    entries.push({
      name,
      flags,
      method,
      compressedSize,
      uncompressedSize,
      localHeaderOffset,
    });
    cursor = extraStart + extraLength + commentLength;
  }
  return entries;
}

/** 从中央目录条目里解出目标条目的内容。 */
export function extractEntry(buffer, entry) {
  const header = entry.localHeaderOffset;
  if (header + 30 > buffer.length || buffer.readUInt32LE(header) !== LOCAL_SIGNATURE) {
    throw new Error(`条目 ${entry.name} 的本地头无效`);
  }
  const nameLength = buffer.readUInt16LE(header + 26);
  const extraLength = buffer.readUInt16LE(header + 28);
  const dataStart = header + 30 + nameLength + extraLength;
  const dataEnd = dataStart + entry.compressedSize;
  if (dataEnd > buffer.length) {
    throw new Error(`条目 ${entry.name} 的数据超出文件范围`);
  }
  const compressed = buffer.subarray(dataStart, dataEnd);
  if (entry.method === 0) {
    return Buffer.from(compressed);
  }
  if (entry.method === 8) {
    return zlib.inflateRawSync(compressed);
  }
  throw new Error(`条目 ${entry.name} 使用了不支持的压缩方式 ${entry.method}`);
}

/** 在文件尾部查找 EOCD；ZIP 注释最长 65535 字节。 */
function findEndOfCentralDirectory(buffer) {
  const start = Math.max(0, buffer.length - EOCD_MIN_SIZE - MAX_COMMENT_SIZE);
  for (let offset = buffer.length - EOCD_MIN_SIZE; offset >= start; offset -= 1) {
    if (buffer.readUInt32LE(offset) === EOCD_SIGNATURE) return offset;
  }
  return -1;
}

/** 存在 ZIP64 结束记录时读取真正的条目数与中央目录位置。 */
function readZip64EndOfCentralDirectory(buffer, eocd) {
  const locator = eocd - 20;
  if (locator < 0 || buffer.readUInt32LE(locator) !== ZIP64_LOCATOR_SIGNATURE) return null;
  const recordOffset = Number(buffer.readBigUInt64LE(locator + 8));
  if (recordOffset + 56 > buffer.length) throw new Error('ZIP64 结束记录超出文件范围');
  if (buffer.readUInt32LE(recordOffset) !== ZIP64_EOCD_SIGNATURE) {
    throw new Error('ZIP64 结束记录签名无效');
  }
  return {
    entryCount: Number(buffer.readBigUInt64LE(recordOffset + 32)),
    directorySize: Number(buffer.readBigUInt64LE(recordOffset + 40)),
    directoryOffset: Number(buffer.readBigUInt64LE(recordOffset + 48)),
  };
}

/** 按需从 0x0001 额外字段里取出被写成 0xffffffff 的尺寸或偏移。 */
function readZip64Extra(extra, wantUncompressed, wantCompressed, wantOffset) {
  let cursor = 0;
  while (cursor + 4 <= extra.length) {
    const id = extra.readUInt16LE(cursor);
    const size = extra.readUInt16LE(cursor + 2);
    const body = extra.subarray(cursor + 4, cursor + 4 + size);
    if (id === ZIP64_EXTRA_ID) {
      let position = 0;
      const result = {};
      if (wantUncompressed && position + 8 <= body.length) {
        result.uncompressedSize = Number(body.readBigUInt64LE(position));
        position += 8;
      }
      if (wantCompressed && position + 8 <= body.length) {
        result.compressedSize = Number(body.readBigUInt64LE(position));
        position += 8;
      }
      if (wantOffset && position + 8 <= body.length) {
        result.localHeaderOffset = Number(body.readBigUInt64LE(position));
      }
      return result;
    }
    cursor += 4 + size;
  }
  return {};
}

/** 按条目名（不区分大小写、忽略路径分隔符差异）查找条目。 */
export function findEntry(entries, wanted) {
  const normalized = normalizeArchivePath(wanted);
  if (!normalized) return null;
  const lowered = normalized.toLowerCase();
  return entries.find((entry) => normalizeArchivePath(entry.name)?.toLowerCase() === lowered) ?? null;
}

/**
 * 归一化压缩包内的路径。
 *
 * 与 CLI 的 normalize_archive_path 一致：反斜杠转正斜杠，拒绝绝对路径、
 * `..` 和含冒号的片段，去掉多余的 `.` 与空片段。
 */
export function normalizeArchivePath(path) {
  const value = String(path ?? '').trim().replaceAll('\\', '/');
  if (!value || value.startsWith('/')) return null;
  const parts = value.split('/');
  if (parts.some((part) => part === '..' || part.includes(':'))) return null;
  return parts.filter((part) => part !== '' && part !== '.').join('/');
}
