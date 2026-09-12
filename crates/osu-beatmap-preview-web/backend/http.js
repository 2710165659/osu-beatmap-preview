// 基于 node:http/https 的最小请求封装。
//
// 使用原生 http 而不是全局 fetch，是因为 osu.direct 的优选 IP 需要按主机名
// 覆盖 DNS 解析（等价于 CLI 里的 ureq Resolver）：https 请求仍以域名做 SNI 与
// 证书校验，只把连接地址换成探测出来的 IP，其他主机继续走系统 DNS。

import http from 'node:http';
import https from 'node:https';
import { lookup as dnsLookup } from 'node:dns';
import { CONNECT_TIMEOUT, READ_TIMEOUT } from './config.js';

const REDIRECT_STATUS = new Set([301, 302, 303, 307, 308]);
const OSZ_DIRECT_HOST = 'osu.direct';

/** 只覆盖 osu.direct 的解析结果，其余主机名回退到系统 DNS。 */
export function createLookup(preferredIp) {
  if (!preferredIp) return undefined;
  return (hostname, options, callback) => {
    if (hostname.toLowerCase() === OSZ_DIRECT_HOST) {
      callback(null, preferredIp, 4);
      return;
    }
    dnsLookup(hostname, options, callback);
  };
}

/**
 * 发起一次 HTTP(S) 请求并按需跟随重定向。
 *
 * 返回值保留原始响应流，由调用方决定是流式落盘还是一次性读入内存。
 */
export async function httpRequest(url, options = {}) {
  const {
    method = 'GET',
    headers = {},
    lookup,
    signal,
    connectTimeoutMs = CONNECT_TIMEOUT,
    idleTimeoutMs = READ_TIMEOUT,
    maxRedirects = 5,
  } = options;

  let current = url instanceof URL ? url : new URL(url);
  for (let hop = 0; ; hop += 1) {
    const response = await singleRequest(current, {
      method,
      headers,
      lookup,
      signal,
      connectTimeoutMs,
      idleTimeoutMs,
    });
    const location = response.headers.location;
    if (REDIRECT_STATUS.has(response.status) && location && hop < maxRedirects) {
      // 重定向响应体没有价值，直接丢弃再跟随；跳转后的主机不再套用优选 IP。
      response.body.resume();
      current = new URL(location, current);
      continue;
    }
    return { ...response, url: current.toString() };
  }
}

function singleRequest(url, { method, headers, lookup, signal, connectTimeoutMs, idleTimeoutMs }) {
  return new Promise((resolve, reject) => {
    if (signal?.aborted) {
      reject(abortError());
      return;
    }
    const transport = url.protocol === 'http:' ? http : https;
    let settled = false;
    // agent: false 让每次请求使用独立的连接，避免 keep-alive 复用与取消语义冲突。
    const request = transport.request(
      url,
      { method, headers, lookup, agent: false },
      (response) => {
        settled = true;
        resolve({
          status: response.statusCode ?? 0,
          headers: response.headers,
          body: response,
        });
      },
    );

    let connectTimer = setTimeout(() => {
      request.destroy(new Error(`连接 ${url.host} 超时（${connectTimeoutMs} ms）`));
    }, connectTimeoutMs);
    const clearConnectTimer = () => {
      clearTimeout(connectTimer);
      connectTimer = null;
    };
    request.on('socket', (socket) => {
      socket.once('connect', clearConnectTimer);
      socket.once('secureConnect', clearConnectTimer);
    });
    request.setTimeout(idleTimeoutMs, () => {
      request.destroy(new Error(`读写 ${url.host} 超时（${idleTimeoutMs} ms）`));
    });

    const onAbort = () => request.destroy(abortError());
    signal?.addEventListener('abort', onAbort, { once: true });

    request.on('error', (error) => {
      clearConnectTimer();
      signal?.removeEventListener('abort', onAbort);
      if (settled) {
        // 响应头已经返回，错误发生在流读取阶段，由调用方通过流错误处理。
        return;
      }
      reject(error);
    });
    request.end();
  });
}

/** 统一的取消错误，调用方据此区分「用户取消」与「下载失败」。 */
export function abortError() {
  const error = new Error('download aborted');
  error.aborted = true;
  return error;
}

export function isAbortError(error) {
  return Boolean(error?.aborted) || error?.name === 'AbortError';
}

/** 读取响应流到内存，并按上限截断保护。 */
export async function readAll(body, { maxBytes = Infinity, onBytes, signal } = {}) {
  const chunks = [];
  let total = 0;
  for await (const chunk of body) {
    if (signal?.aborted) throw abortError();
    total += chunk.length;
    if (total > maxBytes) {
      throw new Error(`响应超过大小上限 ${maxBytes} 字节`);
    }
    onBytes?.(chunk.length);
    chunks.push(chunk);
  }
  return Buffer.concat(chunks, total);
}

/** 丢弃响应体，用于探测类请求。 */
export async function drain(body) {
  for await (const _chunk of body) {
    // 读取即丢弃；探测请求的响应体很小。
  }
}

/** 服务端返回 HTML 说明命中了错误页而不是压缩包。 */
export function assertNotHtml(headers) {
  const contentType = String(headers['content-type'] ?? '').toLowerCase();
  if (contentType.includes('text/html')) {
    throw new Error('服务端返回 HTML 而不是 OSZ 压缩包');
  }
}

/** 判断响应是否支持 Range，并解析出总长度。 */
export function parseContentRange(value) {
  const text = String(value ?? '').trim();
  const match = /^bytes (\d+)-(\d+)\/(\d+)$/.exec(text);
  if (!match) {
    throw new Error(`无效的 Content-Range：'${text}'`);
  }
  const [, start, end, total] = match.map(Number);
  if (start > end || end >= total) {
    throw new Error(`无效的 Content-Range：'${text}'`);
  }
  return { start, end, total };
}
