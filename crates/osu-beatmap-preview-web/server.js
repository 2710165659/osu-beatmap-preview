#!/usr/bin/env node
// osu! 谱面预览 Web 站点与下载后端。
//
// 后端只做浏览器做不到的事：跨域下载 `.osu` 与 `.osz`、从 OSZ 里取出音频和背景、
// 缓存结果，然后把静态站点（含 wasm 产物）发给浏览器。渲染完全在浏览器 WebGPU 中
// 完成，后端不参与任何一帧的绘制。

import http from 'node:http';
import fsp from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { Cache, resolveCacheDir } from './src/cache.js';
import { DEFAULT_HOST, DEFAULT_PORT } from './src/config.js';
import { ResourceProvider } from './src/resources.js';
import { StaticFiles, parseByteRange } from './src/static.js';

const ROOT = path.dirname(fileURLToPath(import.meta.url));
const PUBLIC_DIR = path.join(ROOT, 'public');

const options = parseArgs(process.argv.slice(2));
if (options.help) {
  printUsage();
  process.exit(0);
}

const cache = new Cache(resolveCacheDir({ explicit: options.cacheDir, root: ROOT }));
await cache.ensure();
const staticFiles = new StaticFiles(PUBLIC_DIR);
const resources = new ResourceProvider({ cache, log: options.quiet ? () => {} : log });

await warnIfWasmMissing();

const server = http.createServer((request, response) => {
  handle(request, response).catch((error) => {
    log(`请求失败：${request.method} ${request.url} ${error.message}`);
    if (!response.headersSent) {
      sendText(response, 502, `Bad Gateway: ${error.message}`);
    } else {
      response.destroy();
    }
  });
});

server.listen(options.port, options.host, () => {
  log(`osu! beatmap preview web: http://${options.host}:${options.port}`);
  log(`静态站点目录：${PUBLIC_DIR}`);
  log(`缓存目录：${cache.root}`);
});

async function handle(request, response) {
  const url = new URL(request.url ?? '/', `http://${request.headers.host ?? 'localhost'}`);
  const route = url.pathname;

  if (route === '/' || route === '/index.html' || route === '/app.js' || route === '/style.css') {
    await sendStatic(response, route === '/' ? '/index.html' : route);
    return;
  }
  if (route.startsWith('/pkg/')) {
    await sendStatic(response, route);
    return;
  }
  if (route.startsWith('/resource/')) {
    await sendResource(request, response, route, url.searchParams.get('bid'));
    return;
  }
  sendText(response, 404, 'not found');
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

function parseArgs(argv) {
  const parsed = {
    host: DEFAULT_HOST,
    port: DEFAULT_PORT,
    cacheDir: null,
    noCache: false,
    quiet: false,
    help: false,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    const next = () => {
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

function printUsage() {
  console.log(`用法：node server.js [--host=<ADDR>] [--port=<PORT>] [--cache-dir=<DIR>] [--no-cache] [--quiet]

  --host       监听地址，默认 ${DEFAULT_HOST}
  --port       监听端口，默认 ${DEFAULT_PORT}
  --cache-dir  缓存目录，默认 <安装目录>/.cache，也可用 OSU_PREVIEW_CACHE_DIR 指定
  --no-cache   忽略已缓存的 .osu 与 .osz，强制重新下载
  --quiet      不打印下载日志
  --help       打印本用法`);
}

function log(message) {
  console.log(`[${new Date().toISOString()}] ${message}`);
}

async function warnIfWasmMissing() {
  const wasm = path.join(PUBLIC_DIR, 'pkg', 'osu_beatmap_preview_wasm.js');
  try {
    await fsp.access(wasm);
  } catch {
    log('警告：public/pkg 下没有 wasm 产物，页面无法渲染。请先执行 npm run build:wasm。');
  }
}
