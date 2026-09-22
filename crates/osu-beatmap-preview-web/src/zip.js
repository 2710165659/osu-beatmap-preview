// 浏览器侧的最小 ZIP 读取（`backend/zip.js` 的前端对应实现）。
//
// 本地上传的 `.osz` 在浏览器里解开、不经过后端：中央目录解析与条目归一化规则与
// 后端完全一致（`test/local-zip.test.js` 用同一个 fixture 钉住两边，归一化用例表
// 还与 core 的 `domain/media.rs` 共用），解压则用浏览器原生的
// `DecompressionStream('deflate-raw')`，前端因此不需要任何第三方依赖。

const EOCD_SIGNATURE = 0x06054b50;
const ZIP64_EOCD_SIGNATURE = 0x06064b50;
const ZIP64_LOCATOR_SIGNATURE = 0x07064b50;
const CENTRAL_SIGNATURE = 0x02014b50;
const LOCAL_SIGNATURE = 0x04034b50;
const EOCD_MIN_SIZE = 22;
const MAX_COMMENT_SIZE = 0xffff;
const ZIP64_EXTRA_ID = 0x0001;

const textDecoder = new TextDecoder('utf-8');

/**
 * 解析中央目录，返回条目清单。
 *
 * @param {Uint8Array} bytes 整个压缩包的字节（Buffer 也可以，它是 Uint8Array 的子类）
 */
export function readZipIndex(bytes) {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const eocd = findEndOfCentralDirectory(view);
  if (eocd < 0) throw new Error('缺少 ZIP 中央目录结束记录');

  let entryCount = view.getUint16(eocd + 10, true);
  let directorySize = view.getUint32(eocd + 12, true);
  let directoryOffset = view.getUint32(eocd + 16, true);

  const zip64 = readZip64EndOfCentralDirectory(view, eocd);
  if (zip64) {
    entryCount = zip64.entryCount;
    directorySize = zip64.directorySize;
    directoryOffset = zip64.directoryOffset;
  }

  if (directoryOffset + directorySize > bytes.byteLength) {
    throw new Error('ZIP 中央目录超出文件范围');
  }

  const entries = [];
  let cursor = directoryOffset;
  for (let index = 0; index < entryCount; index += 1) {
    if (cursor + 46 > bytes.byteLength || view.getUint32(cursor, true) !== CENTRAL_SIGNATURE) {
      throw new Error(`第 ${index} 个中央目录条目无效`);
    }
    const flags = view.getUint16(cursor + 8, true);
    const method = view.getUint16(cursor + 10, true);
    let compressedSize = view.getUint32(cursor + 20, true);
    let uncompressedSize = view.getUint32(cursor + 24, true);
    const nameLength = view.getUint16(cursor + 28, true);
    const extraLength = view.getUint16(cursor + 30, true);
    const commentLength = view.getUint16(cursor + 32, true);
    let localHeaderOffset = view.getUint32(cursor + 42, true);
    const nameStart = cursor + 46;
    const extraStart = nameStart + nameLength;
    if (extraStart + extraLength + commentLength > bytes.byteLength) {
      throw new Error(`第 ${index} 个中央目录条目超出文件范围`);
    }
    const name = textDecoder.decode(bytes.subarray(nameStart, extraStart));

    if (
      compressedSize === 0xffffffff ||
      uncompressedSize === 0xffffffff ||
      localHeaderOffset === 0xffffffff
    ) {
      const zip64Values = readZip64Extra(
        view,
        extraStart,
        extraLength,
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

/**
 * 解出目标条目的内容。
 *
 * 方法 0（存储）直接切片，方法 8（deflate）交给 `DecompressionStream`，
 * 因此是异步函数；其余压缩方式与后端一致，直接拒绝。
 *
 * @param {Uint8Array} bytes 整个压缩包的字节
 * @param {{name: string, method: number, compressedSize: number}} entry
 * @returns {Promise<Uint8Array>}
 */
export async function extractEntry(bytes, entry) {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const header = entry.localHeaderOffset;
  if (header + 30 > bytes.byteLength || view.getUint32(header, true) !== LOCAL_SIGNATURE) {
    throw new Error(`条目 ${entry.name} 的本地头无效`);
  }
  const nameLength = view.getUint16(header + 26, true);
  const extraLength = view.getUint16(header + 28, true);
  const dataStart = header + 30 + nameLength + extraLength;
  const dataEnd = dataStart + entry.compressedSize;
  if (dataEnd > bytes.byteLength) {
    throw new Error(`条目 ${entry.name} 的数据超出文件范围`);
  }
  const compressed = bytes.subarray(dataStart, dataEnd);
  if (entry.method === 0) {
    return compressed.slice();
  }
  if (entry.method === 8) {
    return inflateRaw(compressed);
  }
  throw new Error(`条目 ${entry.name} 使用了不支持的压缩方式 ${entry.method}`);
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
 * 与后端 `normalizeArchivePath`、core 的 `normalize_entry_path` 是同一套规则
 * （用例表在 `test/media-contract.test.js` 里三方共用）：反斜杠转正斜杠，拒绝绝对
 * 路径、`..` 和含冒号的片段，去掉多余的 `.` 与空片段；归一化后什么都不剩时返回
 * `null`（而不是空串）。
 */
export function normalizeArchivePath(path) {
  const value = String(path ?? '').trim().replaceAll('\\', '/');
  if (!value || value.startsWith('/')) return null;
  const parts = value.split('/');
  if (parts.some((part) => part === '..' || part.includes(':'))) return null;
  return parts.filter((part) => part !== '' && part !== '.').join('/') || null;
}

/** 在文件尾部查找 EOCD；ZIP 注释最长 65535 字节。 */
function findEndOfCentralDirectory(view) {
  const start = Math.max(0, view.byteLength - EOCD_MIN_SIZE - MAX_COMMENT_SIZE);
  for (let offset = view.byteLength - EOCD_MIN_SIZE; offset >= start; offset -= 1) {
    if (view.getUint32(offset, true) === EOCD_SIGNATURE) return offset;
  }
  return -1;
}

/** 存在 ZIP64 结束记录时读取真正的条目数与中央目录位置。 */
function readZip64EndOfCentralDirectory(view, eocd) {
  const locator = eocd - 20;
  if (locator < 0 || view.getUint32(locator, true) !== ZIP64_LOCATOR_SIGNATURE) return null;
  const recordOffset = Number(view.getBigUint64(locator + 8, true));
  if (recordOffset + 56 > view.byteLength) throw new Error('ZIP64 结束记录超出文件范围');
  if (view.getUint32(recordOffset, true) !== ZIP64_EOCD_SIGNATURE) {
    throw new Error('ZIP64 结束记录签名无效');
  }
  return {
    entryCount: Number(view.getBigUint64(recordOffset + 32, true)),
    directorySize: Number(view.getBigUint64(recordOffset + 40, true)),
    directoryOffset: Number(view.getBigUint64(recordOffset + 48, true)),
  };
}

/** 按需从 0x0001 额外字段里取出被写成 0xffffffff 的尺寸或偏移。 */
function readZip64Extra(view, extraStart, extraLength, wantUncompressed, wantCompressed, wantOffset) {
  let cursor = extraStart;
  const extraEnd = extraStart + extraLength;
  while (cursor + 4 <= extraEnd) {
    const id = view.getUint16(cursor, true);
    const size = view.getUint16(cursor + 2, true);
    const body = cursor + 4;
    if (id === ZIP64_EXTRA_ID) {
      let position = body;
      const result = {};
      if (wantUncompressed && position + 8 <= extraEnd) {
        result.uncompressedSize = Number(view.getBigUint64(position, true));
        position += 8;
      }
      if (wantCompressed && position + 8 <= extraEnd) {
        result.compressedSize = Number(view.getBigUint64(position, true));
        position += 8;
      }
      if (wantOffset && position + 8 <= extraEnd) {
        result.localHeaderOffset = Number(view.getBigUint64(position, true));
      }
      return result;
    }
    cursor += 4 + size;
  }
  return {};
}

/** 用浏览器原生流解 raw deflate；Node 18+ 提供同一 API，测试直接复用本文件。 */
async function inflateRaw(compressed) {
  const stream = new Blob([compressed]).stream().pipeThrough(new DecompressionStream('deflate-raw'));
  return new Uint8Array(await new Response(stream).arrayBuffer());
}
