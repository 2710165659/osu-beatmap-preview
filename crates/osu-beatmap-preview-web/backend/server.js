#!/usr/bin/env node
// osu! 谱面预览 Web 站点与下载后端。
//
// 后端只做浏览器做不到的事：跨域下载 `.osu` 与 `.osz`、从 OSZ 里取出音频和背景、
// 缓存结果，然后把静态站点（含 wasm 产物）发给浏览器。渲染完全在浏览器 WebGPU 中
// 完成，后端不参与任何一帧的绘制。

import http from 'node:http';
import https from 'node:https';
import os from 'node:os';
import { spawnSync } from 'node:child_process';
import fsp from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { Cache, resolveCacheDir } from './cache.js';
import { DEFAULT_HOST, DEFAULT_PORT } from './config.js';
import { ResourceProvider } from './resources.js';
import { StaticFiles, parseByteRange } from './static.js';

// 站点根目录：本文件在 backend/ 下，缓存与构建产物都相对包根目录定位，
// 这样 `node backend/server.js` 仍然使用 <安装目录>/.cache 与 <安装目录>/dist。
const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
// 静态站点由 `npm run build`（Vite）输出到 dist/，源码目录不再直接托管。
const PUBLIC_DIR = path.join(ROOT, 'dist');

const options = parseArgs(process.argv.slice(2));
if (options.help) {
  printUsage();
  process.exit(0);
}

const cache = new Cache(resolveCacheDir({ explicit: options.cacheDir, root: ROOT }));
await cache.ensure();
const staticFiles = new StaticFiles(PUBLIC_DIR);
const resources = new ResourceProvider({ cache, log: options.quiet ? () => {} : log });

await warnIfBuildMissing();

const requestListener = (request, response) => {
  handle(request, response).catch((error) => {
    log(`请求失败：${request.method} ${request.url} ${error.message}`);
    if (!response.headersSent) {
      sendText(response, 502, `Bad Gateway: ${error.message}`);
    } else {
      response.destroy();
    }
  });
};

// WebGPU 只在安全上下文里可用：localhost 算安全上下文，但局域网 IP 必须走 HTTPS，
// 否则手机浏览器里 navigator.gpu 是 undefined。--https 时用自签证书起 TLS。
const scheme = options.https ? 'https' : 'http';
const server = options.https
  ? https.createServer(await loadTlsCredentials(), requestListener)
  : http.createServer(requestListener);

server.listen(options.port, options.host, () => {
  for (const address of listenAddresses(options.host)) {
    log(`osu! beatmap preview web: ${scheme}://${address}:${options.port}`);
  }
  // 容器里上面几行打的是容器自己的网卡地址，浏览器用不了；--tls-host 才是实际访问地址。
  if (options.tlsHosts.length > 0) {
    const urls = options.tlsHosts.map((host) => `${scheme}://${host}`).join('  ');
    log(`浏览器访问这些地址：${urls}（容器内端口 ${options.port}，映射到 443 时 URL 不用写端口）`);
  }
  if (options.https) {
    log('使用的是自签证书：浏览器会提示不安全，点“高级 → 继续访问”即可，之后 WebGPU 才能用。');
  }
  log(`静态站点目录：${PUBLIC_DIR}`);
  log(`缓存目录：${cache.root}`);
});

async function handle(request, response) {
  const url = new URL(request.url ?? '/', `http://${request.headers.host ?? 'localhost'}`);
  const route = url.pathname;

  if (route.startsWith('/resource/')) {
    await sendResource(request, response, route, url.searchParams.get('bid'));
    return;
  }
  // 其余路径都当作静态文件（含 /pkg/ 下的 wasm 产物与 /gpu-check.html），
  // 路径归一化会拒绝越界访问。
  await sendStatic(response, route === '/' ? '/index.html' : route);
}

async function sendStatic(response, route) {
  const file = await staticFiles.read(route);
  if (!file) {
    sendText(response, 404, 'not found');
    return;
  }
  // 前端是固定 URL：禁止浏览器缓存，否则重建后仍会跑旧界面。
  response.writeHead(200, {
    'Content-Type': file.mime,
    'Content-Length': file.body.length,
    'Cache-Control': 'no-store',
  });
  response.end(file.body);
}

async function sendResource(request, response, route, bid) {
  if (!bid || !/^\d+$/.test(bid)) {
    sendText(response, 400, '缺少或非法的 bid');
    return;
  }

  if (route === '/resource/beatmap') {
    const beatmapPath = await resources.beatmapPath(bid, { fresh: options.noCache });
    const body = await fsp.readFile(beatmapPath);
    sendBuffer(response, 'text/plain; charset=utf-8', body, request.headers.range);
    return;
  }

  // 加载进度：.osz 动辄几十 MiB，前端轮询这个接口画进度条。
  if (route === '/resource/progress') {
    sendJson(response, resources.progress(bid));
    return;
  }

  const media = await resources.media(bid, { fresh: options.noCache });
  const target =
    route === '/resource/audio'
      ? media.audio
      : route === '/resource/background'
        ? media.background
        : null;
  if (!target) {
    sendText(response, 404, '未知资源请求');
    return;
  }
  const body = await fsp.readFile(target.path);
  // 音频元素需要 byte range 才能在不完整下载时 seek，缺少它 seekable 会一直是 0。
  sendBuffer(response, target.mime, body, request.headers.range);
}

function sendBuffer(response, contentType, body, rangeHeader) {
  const range = parseByteRange(rangeHeader, body.length);
  if (!range) {
    response.writeHead(200, {
      'Content-Type': contentType,
      'Content-Length': body.length,
      'Accept-Ranges': 'bytes',
      'Cache-Control': 'no-store',
    });
    response.end(body);
    return;
  }
  const slice = body.subarray(range.start, range.end + 1);
  response.writeHead(206, {
    'Content-Type': contentType,
    'Content-Length': slice.length,
    'Accept-Ranges': 'bytes',
    'Content-Range': `bytes ${range.start}-${range.end}/${body.length}`,
    'Cache-Control': 'no-store',
  });
  response.end(slice);
}

function sendText(response, status, text) {
  response.writeHead(status, {
    'Content-Type': 'text/plain; charset=utf-8',
    'Content-Length': Buffer.byteLength(text),
    'Cache-Control': 'no-store',
  });
  response.end(text);
}

/** 进度查询是短轮询接口：不能缓存，否则前端永远读到同一份快照。 */
function sendJson(response, payload) {
  const body = Buffer.from(JSON.stringify(payload));
  response.writeHead(200, {
    'Content-Type': 'application/json; charset=utf-8',
    'Content-Length': body.length,
    'Cache-Control': 'no-store',
  });
  response.end(body);
}

function parseArgs(argv) {
  const parsed = {
    host: DEFAULT_HOST,
    port: DEFAULT_PORT,
    cacheDir: null,
    noCache: false,
    quiet: false,
    https: false,
    tlsCert: null,
    tlsKey: null,
    // 自签证书要覆盖的地址：容器里探测到的 IP 是容器自己的，必须能手动补上。
    tlsHosts: extraTlsHosts(),
    help: false,
  };
  for (let index = 0; index < argv.length; index += 1) {
    // 用法里写的是 `--cache-dir=<DIR>`，所以 `--flag=value` 和 `--flag value` 都要认。
    const [argument, inlineValue] = splitArgument(argv[index]);
    const next = () => {
      if (inlineValue !== undefined) return inlineValue;
      const value = argv[index + 1];
      if (value === undefined) throw new Error(`${argument} 需要一个值`);
      index += 1;
      return value;
    };
    switch (argument) {
      case '--host':
        parsed.host = next();
        break;
      case '--port':
        parsed.port = Number(next());
        if (!Number.isInteger(parsed.port) || parsed.port <= 0 || parsed.port > 65535) {
          throw new Error('--port 必须是 1-65535 的整数');
        }
        break;
      case '--cache-dir':
        parsed.cacheDir = next();
        break;
      case '--no-cache':
        parsed.noCache = true;
        break;
      case '--quiet':
        parsed.quiet = true;
        break;
      case '--https':
        parsed.https = true;
        break;
      case '--tls-cert':
        parsed.tlsCert = next();
        parsed.https = true;
        break;
      case '--tls-key':
        parsed.tlsKey = next();
        parsed.https = true;
        break;
      case '--tls-host':
        // 可重复：域名写成 DNS，IPv4 写成 IP，用作自签证书的 SAN。
        parsed.tlsHosts.push(next());
        parsed.https = true;
        break;
      case '--help':
      case '-h':
        parsed.help = true;
        break;
      default:
        throw new Error(`未知参数：${argument}`);
    }
  }
  return parsed;
}

/** 把 `--name=value` 拆成名字与内联值；没有 `=` 时内联值为 undefined。 */
function splitArgument(text) {
  const separator = text.indexOf('=');
  if (separator < 0) return [text, undefined];
  return [text.slice(0, separator), text.slice(separator + 1)];
}

function printUsage() {
  console.log(`用法：node server.js [--host=<ADDR>] [--port=<PORT>] [--cache-dir=<DIR>] [--https] [--tls-cert=<PEM> --tls-key=<PEM>] [--tls-host=<HOST>] [--no-cache] [--quiet]

  --host       监听地址，默认 ${DEFAULT_HOST}；手机同网测试用 --host=0.0.0.0
  --port       监听端口，默认 ${DEFAULT_PORT}
  --cache-dir  缓存目录，默认 <安装目录>/.cache，也可用 OSU_PREVIEW_CACHE_DIR 指定
  --https      启用自签 HTTPS；WebGPU 只在安全上下文可用，局域网/服务器访问需要它
  --tls-cert   HTTPS 证书（PEM），与 --tls-key 一起使用时不再自动生成
  --tls-key    HTTPS 私钥（PEM）
  --tls-host   自签证书要覆盖的域名或 IP（可重复）；容器里探测到的是容器自己的地址，
               必须用它补上浏览器实际访问的域名/IP，也可用 OSU_PREVIEW_TLS_HOSTS 指定
  --no-cache   忽略已缓存的 .osu 与 .osz，强制重新下载
  --quiet      不打印下载日志
  --help       打印本用法`);
}

/** 监听所有网卡时，把本机的局域网地址也打印出来，方便手机直接输入。 */
function listenAddresses(host) {
  if (host !== '0.0.0.0' && host !== '::') return [host];
  return ['127.0.0.1', ...localIPv4Addresses()];
}

function localIPv4Addresses() {
  return Object.values(os.networkInterfaces())
    .flat()
    .filter((entry) => entry && entry.family === 'IPv4' && !entry.internal)
    .map((entry) => entry.address);
}

/** 环境变量 OSU_PREVIEW_TLS_HOSTS（逗号分隔）里的额外地址。 */
function extraTlsHosts() {
  return (process.env.OSU_PREVIEW_TLS_HOSTS ?? '')
    .split(',')
    .map((value) => value.trim())
    .filter(Boolean);
}

/**
 * 自签证书的 SAN 列表。
 *
 * 容器里探测到的网卡地址是容器自己的（172.17.x.x），浏览器根本不会用它访问，
 * 所以要能用 --tls-host / OSU_PREVIEW_TLS_HOSTS 补上实际访问的域名或 IP。
 */
function tlsSubjectAltNames(extraHosts) {
  const names = ['DNS:localhost', 'IP:127.0.0.1'];
  const push = (value) => {
    const host = value.trim();
    if (!host) return;
    const name = /^\d{1,3}(\.\d{1,3}){3}$/.test(host) ? `IP:${host}` : `DNS:${host}`;
    if (!names.includes(name)) names.push(name);
  };
  for (const address of localIPv4Addresses()) push(address);
  for (const host of extraHosts) push(host);
  return names;
}

/** 取用 --tls-cert/--tls-key，或生成本地自签证书（缓存在 <缓存目录>/.tls）。 */
async function loadTlsCredentials() {
  if (options.tlsCert && options.tlsKey) {
    return {
      cert: await fsp.readFile(options.tlsCert),
      key: await fsp.readFile(options.tlsKey),
    };
  }
  if (options.tlsCert || options.tlsKey) {
    throw new Error('--tls-cert 与 --tls-key 必须同时提供');
  }
  return ensureSelfSignedCertificate();
}

async function ensureSelfSignedCertificate() {
  const directory = path.join(cache.root, '.tls');
  const certPath = path.join(directory, 'cert.pem');
  const keyPath = path.join(directory, 'key.pem');
  // 记录签发时的 SAN：换成别的域名/IP 访问时要重新生成，否则缓存下来的证书对不上。
  const hostsPath = path.join(directory, 'hosts.txt');
  const names = tlsSubjectAltNames(options.tlsHosts);
  const signature = names.join(',');
  try {
    const [cert, key, cached] = await Promise.all([
      fsp.readFile(certPath),
      fsp.readFile(keyPath),
      fsp.readFile(hostsPath, 'utf8'),
    ]);
    if (cached.trim() === signature) return { cert, key };
    log('自签证书的地址列表已变化，重新生成。');
  } catch {
    // 首次运行或证书被删除时重新生成。
  }
  await fsp.mkdir(directory, { recursive: true });
  const result = spawnSync(
    'openssl',
    [
      'req',
      '-x509',
      '-newkey',
      'rsa:2048',
      '-nodes',
      '-days',
      '825',
      '-keyout',
      keyPath,
      '-out',
      certPath,
      '-subj',
      '/CN=osu-beatmap-preview',
      '-addext',
      `subjectAltName=${signature}`,
    ],
    { stdio: 'ignore' },
  );
  if (result.error || result.status !== 0) {
    throw new Error(
      '生成自签证书失败：需要 openssl 在 PATH 中，或用 --tls-cert/--tls-key 指定已有证书',
    );
  }
  await fsp.writeFile(hostsPath, `${signature}\n`);
  log(`已生成自签证书：${certPath}`);
  log(`证书覆盖的地址：${signature}`);
  return { cert: await fsp.readFile(certPath), key: await fsp.readFile(keyPath) };
}

function log(message) {
  console.log(`[${new Date().toISOString()}] ${message}`);
}

async function warnIfBuildMissing() {
  const exists = async (target) => {
    try {
      await fsp.access(target);
      return true;
    } catch {
      return false;
    }
  };
  if (!(await exists(path.join(PUBLIC_DIR, 'index.html')))) {
    log('警告：dist/ 下没有构建产物，站点无法打开。请先执行 npm install && npm run build。');
    return;
  }
  if (!(await exists(path.join(PUBLIC_DIR, 'pkg', 'osu_beatmap_preview_wasm.js')))) {
    log('警告：dist/pkg 下没有 wasm 产物，页面无法渲染。请先执行 npm run build:wasm 再重新构建。');
  }
}
