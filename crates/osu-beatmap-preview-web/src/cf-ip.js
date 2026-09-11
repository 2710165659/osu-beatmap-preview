// osu.direct 的 Cloudflare 优选 IP 探测与缓存。
//
// 从 Cloudflare 官方网段里采样候选 IP，先做 TCP 握手测速，再对最快的若干候选
// 发起一次 HTTPS 请求复测，胜者写入缓存并在 CACHE_TTL_SECONDS 内复用。

import net from 'node:net';
import {
  CLOUDFLARE_IPV4_RANGES,
  IP_HTTP_CANDIDATES,
  IP_HTTP_TIMEOUT,
  IP_PROBE_USER_AGENT,
  IP_TCP_TIMEOUT,
} from './config.js';
import { httpRequest, createLookup, drain } from './http.js';

let refreshInFlight = null;

/**
 * 触发一次后台刷新。
 *
 * 已有缓存时直接返回缓存；没有缓存或 force 为真时异步探测，探测期间所有调用
 * 共享同一个 Promise，避免并发请求重复测速。
 */
export async function ensurePreferredIp(cache, { log, force = false } = {}) {
  if (!force) {
    const cached = await cache.readPreferredIp();
    if (cached) return cached;
  }
  if (!refreshInFlight) {
    refreshInFlight = refreshPreferredIp(cache, { log, force }).finally(() => {
      refreshInFlight = null;
    });
  }
  return refreshInFlight;
}

/** 重新探测并写入缓存；已有锁时说明别的请求正在探测，直接返回当前缓存。 */
export async function refreshPreferredIp(cache, { log } = {}) {
  const release = await cache.acquirePreferredIpLock();
  if (!release) return cache.readPreferredIp();
  try {
    const candidates = await probeTcpCandidates();
    const finalists = candidates
      .sort((left, right) => left.latency - right.latency)
      .slice(0, IP_HTTP_CANDIDATES);
    const winner = await probeHttpCandidates(finalists);
    if (!winner) {
      log?.('osu.direct 优选 IP：没有可用的 Cloudflare 地址');
      return null;
    }
    await cache.writePreferredIp(winner.ip);
    log?.(
      `osu.direct 优选 IP：${winner.ip}（${Math.round(winner.latency)} ms HTTPS）`,
    );
    return winner.ip;
  } catch (error) {
    log?.(`osu.direct 优选 IP 探测失败：${error.message}`);
    return null;
  } finally {
    await release();
  }
}

async function probeTcpCandidates() {
  const results = await Promise.all(
    buildCandidates().map(async (ip) => {
      const started = performance.now();
      try {
        await tcpProbe(ip);
        return { ip, latency: performance.now() - started };
      } catch {
        return null;
      }
    }),
  );
  return results.filter(Boolean);
}

function tcpProbe(ip) {
  return new Promise((resolve, reject) => {
    const socket = net.connect({ host: ip, port: 443, timeout: IP_TCP_TIMEOUT });
    const fail = () => {
      socket.destroy();
      reject(new Error('tcp probe failed'));
    };
    socket.once('connect', () => {
      socket.destroy();
      resolve();
    });
    socket.once('timeout', fail);
    socket.once('error', fail);
  });
}

async function probeHttpCandidates(candidates) {
  const results = await Promise.all(
    candidates.map(async ({ ip }) => {
      const started = performance.now();
      try {
        const response = await httpRequest('https://osu.direct/', {
          headers: { 'User-Agent': IP_PROBE_USER_AGENT },
          lookup: createLookup(ip),
          connectTimeoutMs: IP_HTTP_TIMEOUT,
          idleTimeoutMs: IP_HTTP_TIMEOUT,
        });
        await drain(response.body);
        if (response.status >= 500) return null;
        return { ip, latency: performance.now() - started };
      } catch {
        return null;
      }
    }),
  );
  const usable = results.filter(Boolean);
  return usable.sort((left, right) => left.latency - right.latency)[0] ?? null;
}

function buildCandidates() {
  const seed = (Math.floor(Date.now() / 1000) ^ process.pid) >>> 0;
  const candidates = [];
  CLOUDFLARE_IPV4_RANGES.forEach((range, index) => {
    candidates.push(...sampleRange(range, (seed + index) >>> 0));
  });
  return candidates;
}

/** 在每个网段内按种子取两个地址，避免每次都测同一批 IP。 */
function sampleRange(cidr, seed) {
  const [network, prefixText] = cidr.split('/');
  const prefix = Number(prefixText);
  const base = ipv4ToInt(network);
  if (!Number.isFinite(prefix) || prefix > 32 || base == null) return [];
  const masked = prefix === 0 ? 0 : (base & (0xffffffff << (32 - prefix))) >>> 0;
  const hostCount = 2 ** (32 - prefix);
  const maxOffset = Math.max(hostCount - 1, 1);
  const first = 1 + (seed % maxOffset);
  const second = 1 + (rotateLeft(seed, 13) % maxOffset);
  return [intToIpv4((masked + first) >>> 0), intToIpv4((masked + second) >>> 0)];
}

function ipv4ToInt(text) {
  const parts = String(text).split('.');
  if (parts.length !== 4) return null;
  let value = 0;
  for (const part of parts) {
    const byte = Number(part);
    if (!Number.isInteger(byte) || byte < 0 || byte > 255) return null;
    value = ((value << 8) | byte) >>> 0;
  }
  return value;
}

function intToIpv4(value) {
  return [(value >>> 24) & 0xff, (value >>> 16) & 0xff, (value >>> 8) & 0xff, value & 0xff].join(
    '.',
  );
}

function rotateLeft(value, bits) {
  return ((value << bits) | (value >>> (32 - bits))) >>> 0;
}
