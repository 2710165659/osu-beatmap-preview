// 下载与缓存相关常量。数值与 CLI 的 CLI 专用配置（cli_config.yml）保持一致，
// 便于两个前端复用同一套下载行为；单位在名称中标注。

/** 单次 OSZ 下载允许的最大字节数（50 MiB）。 */
export const MAX_OSZ_BYTES = 50 * 1024 * 1024;
/** OSZ 分块并行下载的块数。 */
export const PARALLEL_PARTS = 4;
/** 同一下载任务允许同时进行的最大尝试数。 */
export const MAX_ACTIVE_ATTEMPTS = 3;
/** 单次完整下载的硬超时（毫秒）。 */
export const DOWNLOAD_HARD_TIMEOUT = 120_000;
/** 单次请求的下载截止时间（毫秒）；由宿主请求触发，超时后取消全部尝试。 */
export const REQUEST_DEADLINE = 300_000;
/** 下载请求使用的 User-Agent。 */
export const USER_AGENT = 'osu-beatmap-preview/1.0';
/** 优选 IP 探测使用的 User-Agent。 */
export const IP_PROBE_USER_AGENT = 'osu-beatmap-preview-cf-speed-test';

/** 读写缓冲区大小（字节）。 */
export const BUFFER_SIZE = 64 * 1024;
/** 下载状态轮询间隔（毫秒）。 */
export const POLL_INTERVAL = 250;
/** 首字节等待超时（毫秒）。 */
export const NO_FIRST_BYTE_TIMEOUT = 3_000;
/** 低速检测窗口时长（毫秒）。 */
export const LOW_SPEED_WINDOW = 5_000;
/** 低速阈值（字节/秒）。 */
export const LOW_SPEED_BYTES_PER_SECOND = 128 * 1024;
/** TCP 连接超时（毫秒）。 */
export const CONNECT_TIMEOUT = 10_000;
/** HTTP 读取空闲超时（毫秒）。 */
export const READ_TIMEOUT = 15_000;
/** HTTP 写入超时（毫秒）。 */
export const WRITE_TIMEOUT = 10_000;
/** 单次 .osu 下载超时（毫秒）。 */
export const OSU_REQUEST_TIMEOUT = 20_000;

/** 优选 IP 缓存有效期（秒）。 */
export const CACHE_TTL_SECONDS = 86_400;
/** 优选 IP 探测的 TCP 连接超时（毫秒）。 */
export const IP_TCP_TIMEOUT = 1_200;
/** 优选 IP 探测的 HTTPS 超时（毫秒）。 */
export const IP_HTTP_TIMEOUT = 2_500;
/** 进入 HTTPS 复测的候选数量。 */
export const IP_HTTP_CANDIDATES = 8;
/** 优选 IP 缓存锁的过期时间（毫秒）。 */
export const IP_LOCK_STALE = 60_000;

/** Cloudflare 官方 IPv4 网段列表，用于采样 osu.direct 的优选 IP。 */
export const CLOUDFLARE_IPV4_RANGES = [
  '173.245.48.0/20',
  '103.21.244.0/22',
  '103.22.200.0/22',
  '103.31.4.0/22',
  '141.101.64.0/18',
  '108.162.192.0/18',
  '190.93.240.0/20',
  '188.114.96.0/20',
  '197.234.240.0/22',
  '198.41.128.0/17',
  '162.158.0.0/15',
  '104.16.0.0/12',
  '172.64.0.0/17',
  '172.64.128.0/18',
  '172.64.192.0/19',
  '172.64.224.0/22',
  '172.64.229.0/24',
  '172.64.230.0/23',
  '172.64.232.0/21',
  '172.64.240.0/21',
  '172.64.248.0/21',
  '172.65.0.0/16',
  '172.66.0.0/16',
  '172.67.0.0/16',
  '131.0.72.0/22',
];

/** OSZ 镜像候选，顺序即降级顺序；osu.direct 的优选 IP 候选会在运行时插入。 */
export const MIRRORS = {
  sayobot: (setId) => `https://txy1.sayobot.cn/beatmaps/download/novideo/${setId}`,
  osuDirect: (setId) => `https://osu.direct/api/d/${setId}`,
  nekoha: (setId) => `https://mirror.nekoha.moe/api/download/${setId}?noVideo=1`,
  catboy: (setId) => `https://catboy.best/d/${setId}`,
};

/** 默认监听端口与地址。 */
export const DEFAULT_PORT = 8787;
export const DEFAULT_HOST = '127.0.0.1';

/** MIME 表：静态资源与压缩包内媒体。 */
export const MIME_TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'application/javascript; charset=utf-8',
  '.mjs': 'application/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.wasm': 'application/wasm',
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.jpeg': 'image/jpeg',
  '.webp': 'image/webp',
  '.svg': 'image/svg+xml',
  '.ico': 'image/x-icon',
  '.mp3': 'audio/mpeg',
  '.ogg': 'audio/ogg',
  '.wav': 'audio/wav',
  '.m4a': 'audio/mp4',
  '.mp4': 'audio/mp4',
  '.aac': 'audio/aac',
  '.flac': 'audio/flac',
};

/** 按扩展名返回 MIME，未知扩展名回退为二进制流。 */
export function mimeFor(name) {
  const dot = name.lastIndexOf('.');
  if (dot < 0) return 'application/octet-stream';
  return MIME_TYPES[name.slice(dot).toLowerCase()] ?? 'application/octet-stream';
}
