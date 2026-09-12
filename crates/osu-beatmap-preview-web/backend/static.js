// 静态站点文件读取。
//
// 只服务 public/ 目录内的普通文件；所有路径都做归一化，拒绝 `..`、绝对路径和
// 盘符，避免把安装目录外的文件暴露出去。

import fsp from 'node:fs/promises';
import path from 'node:path';
import { mimeFor } from './config.js';

export class StaticFiles {
  constructor(root) {
    this.root = path.resolve(root);
  }

  async read(urlPath) {
    const relative = safeRelativePath(urlPath);
    if (relative === null) return null;
    const target = path.resolve(this.root, relative);
    if (target !== this.root && !target.startsWith(this.root + path.sep)) return null;
    const stat = await fsp.stat(target).catch(() => null);
    if (!stat?.isFile()) return null;
    return { path: target, body: await fsp.readFile(target), mime: mimeFor(target) };
  }
}

/** 把 URL 路径转成相对路径；非法路径返回 null。 */
export function safeRelativePath(urlPath) {
  const decoded = (() => {
    try {
      return decodeURIComponent(urlPath);
    } catch {
      return null;
    }
  })();
  if (decoded === null) return null;
  const trimmed = decoded.replaceAll('\\', '/').replace(/^\/+/, '');
  if (!trimmed) return 'index.html';
  if (trimmed.includes('\0') || /^[a-zA-Z]:/.test(trimmed)) return null;
  const parts = trimmed.split('/');
  if (parts.some((part) => part === '..')) return null;
  return parts.filter((part) => part !== '' && part !== '.').join('/');
}

/** 解析单段 Range 请求头；不支持时返回 null，由调用方回退整文件响应。 */
export function parseByteRange(value, length) {
  if (!value || length === 0) return null;
  const text = String(value).trim();
  if (!text.startsWith('bytes=')) return null;
  const first = text.slice('bytes='.length).split(',')[0];
  const separator = first.indexOf('-');
  if (separator < 0) return null;
  const startText = first.slice(0, separator).trim();
  const endText = first.slice(separator + 1).trim();

  if (startText === '') {
    const suffix = Number(endText);
    if (!Number.isInteger(suffix) || suffix <= 0) return null;
    const size = Math.min(suffix, length);
    return { start: length - size, end: length - 1 };
  }
  const start = Number(startText);
  if (!Number.isInteger(start) || start < 0 || start >= length) return null;
  const end = endText === '' ? length - 1 : Number(endText);
  if (!Number.isInteger(end) || end < start) return null;
  return { start, end: Math.min(end, length - 1) };
}
