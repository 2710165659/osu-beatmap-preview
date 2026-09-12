// 缓存目录布局与原子写入。
//
// 目录结构与 CLI 保持一致（osu-download-cache / osz-download-cache / 优选 IP 缓存），
// 这样同一台机器上的 CLI 与 Web 可以直接复用已经下载好的文件。

import fs from 'node:fs';
import fsp from 'node:fs/promises';
import path from 'node:path';
import { CACHE_TTL_SECONDS, IP_LOCK_STALE } from './config.js';

const OSU_CACHE_DIR = 'osu-download-cache';
const OSZ_CACHE_DIR = 'osz-download-cache';
const MEDIA_CACHE_DIR = 'media';
const PREFERRED_IP_FILE = 'osu-direct-preferred-ip.json';
const PREFERRED_IP_LOCK = 'osu-direct-preferred-ip.json.lock';

/** 解析缓存根目录：显式参数 > 环境变量 > 项目/安装目录下的 .cache。 */
export function resolveCacheDir({ explicit, root }) {
  const configured = explicit ?? process.env.OSU_PREVIEW_CACHE_DIR;
  if (configured) return path.resolve(configured);
  return path.join(root, '.cache');
}

export class Cache {
  constructor(root) {
    this.root = root;
    this.osuDir = path.join(root, OSU_CACHE_DIR);
    this.oszDir = path.join(root, OSZ_CACHE_DIR);
    this.mediaDir = path.join(root, MEDIA_CACHE_DIR);
    this.preferredIpPath = path.join(root, PREFERRED_IP_FILE);
    this.preferredIpLockPath = path.join(root, PREFERRED_IP_LOCK);
  }

  async ensure() {
    await fsp.mkdir(this.osuDir, { recursive: true });
    await fsp.mkdir(this.oszDir, { recursive: true });
    await fsp.mkdir(this.mediaDir, { recursive: true });
  }

  osuPath(bid) {
    return path.join(this.osuDir, `${bid}.osu`);
  }

  oszPath(setId) {
    return path.join(this.oszDir, `${setId}.osz`);
  }

  mediaPath(bid, stem, extension) {
    return path.join(this.mediaDir, bid, `${stem}.${extension}`);
  }

  /** 读取优选 IP；缓存过期、内容非法或时间戳异常时返回 null。 */
  async readPreferredIp() {
    let text;
    try {
      text = await fsp.readFile(this.preferredIpPath, 'utf8');
    } catch {
      return null;
    }
    try {
      const value = JSON.parse(text);
      const ip = String(value?.ip ?? '');
      const testedAt = Number(value?.tested_at);
      if (!/^\d+\.\d+\.\d+\.\d+$/.test(ip) || !Number.isFinite(testedAt)) return null;
      const now = Math.floor(Date.now() / 1000);
      if (now < testedAt || now - testedAt > CACHE_TTL_SECONDS) return null;
      return ip;
    } catch {
      return null;
    }
  }

  async writePreferredIp(ip) {
    const payload = JSON.stringify({ ip, tested_at: Math.floor(Date.now() / 1000) });
    await writeFileAtomic(this.preferredIpPath, payload);
  }

  async invalidatePreferredIp() {
    await fsp.rm(this.preferredIpPath, { force: true });
  }

  /**
   * 抢占优选 IP 刷新锁，避免多个请求同时探测。
   *
   * 返回释放函数；已经有其他进程/请求在探测时返回 null。锁文件超过
   * IP_LOCK_STALE 视为残留并强制接管。
   */
  async acquirePreferredIpLock() {
    for (let attempt = 0; attempt < 2; attempt += 1) {
      try {
        const handle = await fsp.open(this.preferredIpLockPath, 'wx');
        await handle.close();
        return async () => {
          await fsp.rm(this.preferredIpLockPath, { force: true });
        };
      } catch (error) {
        if (error.code !== 'EEXIST' || attempt > 0) {
          if (error.code === 'EEXIST') return null;
          throw error;
        }
        let stale = false;
        try {
          const stat = await fsp.stat(this.preferredIpLockPath);
          stale = Date.now() - stat.mtimeMs > IP_LOCK_STALE;
        } catch {
          stale = true;
        }
        if (!stale) return null;
        await fsp.rm(this.preferredIpLockPath, { force: true });
      }
    }
    return null;
  }
}

/** 先写同目录临时文件再改名，避免中断时留下半个文件。 */
export async function writeFileAtomic(target, data) {
  const directory = path.dirname(target);
  await fsp.mkdir(directory, { recursive: true });
  const temporary = `${target}.${process.pid}.tmp`;
  await fsp.writeFile(temporary, data);
  try {
    await fsp.rename(temporary, target);
  } catch (error) {
    if (process.platform === 'win32') {
      // Windows 上目标文件被占用时 rename 会失败，先删除再改名。
      await fsp.rm(target, { force: true });
      await fsp.rename(temporary, target);
      return;
    }
    await fsp.rm(temporary, { force: true });
    throw error;
  }
}

/** 同步判断文件是否存在且非空。 */
export function existsNonEmpty(target) {
  try {
    const stat = fs.statSync(target);
    return stat.isFile() && stat.size > 0;
  } catch {
    return false;
  }
}
