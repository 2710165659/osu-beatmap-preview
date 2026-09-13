// 预览页的共享状态与全部播放逻辑。
//
// 组件只负责画界面：谱面会话（WASM）、音频元素、播放时钟、Mod 与分辨率切换都集中在
// 这里。state 做成模块级单例，组件直接引用，避免为了几个控件层层传递 props。

import { computed, nextTick, reactive } from 'vue';

// ---------------------------------------------------------------------------
// 常量
// ---------------------------------------------------------------------------

/** 输出分辨率只影响画布尺寸和合成阶段，谱面与 Mod 解析仍在 WASM/core 中完成。 */
export const RESOLUTIONS = Object.freeze({
  1080: { width: 1920, height: 1080 },
  720: { width: 1280, height: 720 },
  480: { width: 854, height: 480 },
});

/** 可选渲染帧率；120 FPS 只在高刷屏上生效，默认 60 FPS。 */
export const FPS_CHOICES = [30, 60, 120];
export const SPEED_CHOICES = [0.5, 0.75, 1, 1.5, 2];

/** 允许的转谱模式；也用于校验 URL 里的 convert 参数。 */
export const CONVERT_MODES = ['standard', 'taiko', 'catch', 'mania'];

/** README「Mod 支持」表：Standard / Taiko / Catch / Mania 的 GIF & MP4 支持项。 */
export const MOD_OPTIONS = Object.freeze({
  standard: ['EZ', 'HR', 'HD', 'DA', 'TC', 'DT', 'HT'],
  taiko: ['EZ', 'HR', 'SW', 'CS', 'DT', 'HT'],
  catch: ['EZ', 'HR', 'DT', 'HT'],
  mania: ['CS', 'DT', 'HT', '1K', '2K', '3K', '4K', '5K', '6K', '7K', '8K', '9K', '10K', 'DS', 'IN', 'HO'],
});

// 播放页沿用 osu! 视频预览的默认背景暗化：暗化 70%，保留 30% 亮度，
// 只处理背景资源，不改变 playfield 和其他平台的渲染配置。
const BACKGROUND_DIM = 0.7;

/** 音频被自动播放策略拦下后重试播放的最小间隔，避免每帧都调用 play()。 */
const AUDIO_RETRY_INTERVAL = 500;

/** 手机浏览器上 seek 要几秒才落地，超时后放行，靠对齐逻辑兜底。 */
const AUDIO_SEEK_TIMEOUT = 2000;
/**
 * 音频与画面差距超过这个值才判定为「需要重新 seek」。
 *
 * 移动端一次 seek 会重新缓冲并卡一下声音，所以微小差距宁可让画面等音频
 * （见 tick 里的对齐），也不要频繁动 seek——反复 seek 正是音画不同步的根源。
 */
const AUDIO_RESYNC_THRESHOLD = 120;
/** 画面时钟最多等音频这么久，超过就退回按墙钟推进。 */
const AUDIO_RESYNC_MAX_WAIT = 1500;
/**
 * 暂停时允许把画面进度对齐到音频的最大偏移。
 *
 * 取 80ms（约 5 帧 @60FPS）是有意的：自动播放被拦下时，用户点画面会先在手势里
 * 恢复播放、紧接着这次点击又把它暂停，音频在这几十毫秒里已经往前走了一点。对齐
 * 窗口放到几百毫秒，就会把这几十毫秒的「向前跳」画出来——那正是用户看到的跳帧。
 */
const PAUSE_ALIGN_TOLERANCE = 80;

/** 触发播放需要「用户激活」，因此只用指针 / 触摸 / 键盘这类真实输入当作手势。 */
const GESTURE_EVENTS = ['pointerdown', 'touchstart', 'keydown'];

/** 手势恢复音频后，多短时间内的播放/暂停操作算「只是想把声音打开」。 */
const GESTURE_RESUME_WINDOW = 400;

/**
 * 音频元数据的等待上限。
 *
 * 极少数环境下媒体元素既不触发 loadedmetadata 也不触发 error（例如拿不到音频输出
 * 设备、或服务端不支持 Range 时的媒体管线卡死）。没有这个上限，loadPreview 的
 * Promise.allSettled 会永远挂住，整个播放页都进不去，因此超时后按「无音频」处理。
 */
const AUDIO_METADATA_TIMEOUT = 15000;

/** 加载进度的轮询间隔；后端只有在真正下发资源时才知道字节数。 */
const PROGRESS_INTERVAL = 400;

/** 进度条淡出的等待时间：鼠标停下大约 1 秒后隐藏。 */
const TIMELINE_HIDE_DELAY = 1000;

/** 方向键按住多久算长按；短于这个时长的按下在松开时按 ±5 秒跳转。 */
const ARROW_HOLD_DELAY = 250;
/** 长按右键的倍速（相对谱面自身速度）。 */
const FAST_FORWARD_SPEED = 3;
/** 长按左键倒带：每 REWIND_INTERVAL 毫秒后退 REWIND_STEP 毫秒。 */
const REWIND_INTERVAL = 120;
const REWIND_STEP = 500;

/** wasm 胶水代码必须和 .wasm 同目录加载，所以按运行时 URL 请求而不是打进 bundle。 */
const WASM_URL = `${import.meta.env.BASE_URL}pkg/osu_beatmap_preview_wasm.js`;

// ---------------------------------------------------------------------------
// 状态
// ---------------------------------------------------------------------------

export const state = reactive({
  page: 'load',
  bid: '',
  convert: '',
  loading: false,
  loadLogs: [],
  playLogs: [],
  mode: 'READY',
  modeKey: 'standard',
  status: '',
  playing: false,
  position: 0,
  duration: 1,
  rendered: false,
  renderError: '',
  // 默认 60 FPS：120 FPS 只在高刷新率屏幕上才有实际区别，留给用户手动开。
  fps: 60,
  resolution: '720',
  speed: 1,
  /** 界面上勾选的 Mod token；DA 提交时会展开成 DAAR..CS..。 */
  mods: [],
  daAr: 9,
  daCs: 4,
  sheetOpen: false,
  /**
   * 自动播放是否被浏览器拦下。
   *
   * 这个值描述的是「此刻音频真的没在响」，而不是「曾经失败过一次」：手势恢复
   * 成功或音频真正开始播放后必须归位，否则静音提示会一直挂着。
   */
  audioBlocked: false,
  /** 进度条是否可见：隐藏时只改透明度，盒子留在原处，画面不会上下跳。 */
  timelineVisible: true,
  /** 长按方向键的状态，用于在画面上给出反馈。 */
  fastForwarding: false,
  rewinding: false,
  /** WASM 解析出的谱面内部信息（`beatmapInfo` 全量输出）；未加载时为 null。 */
  info: null,
  /**
   * 加载进度。
   *
   * `received/total` 是当前阶段的字节数，`total` 为 0 表示还不知道总量（不确定态）；
   * `speed/eta` 由前端对两次轮询采样算出来，服务端不必关心。阶段语义：
   * osu 取谱面 → osz 服务端下载 → extract 服务端解包 → transfer 传给浏览器 →
   * media 浏览器收音频/背景 → ready 进播放页。
   */
  progress: { phase: 'idle', received: 0, total: 0, message: '', speed: 0, eta: null },
  /** 音频与背景是否还在后台准备（进了播放页也可能还没拉完）。 */
  preparingMedia: false,
  gpuAvailable: typeof navigator !== 'undefined' && Boolean(navigator.gpu),
  /** 安全上下文（https / localhost）。WebGPU 只在这里可用，用它区分两种失败原因。 */
  secureContext: typeof window === 'undefined' || window.isSecureContext !== false,
});

/** 加载阶段的文案；未知阶段一律显示「正在加载」。 */
const PROGRESS_LABELS = {
  osu: '获取谱面',
  osz: '服务端下载谱面包',
  extract: '服务端解包音频与背景',
  transfer: '传输到客户端',
  media: '下载音频与背景',
  ready: '准备渲染',
};
const DEFAULT_PROGRESS_LABEL = '正在加载';

/** 标签优先用服务端给的 message（含镜像名等细节），没有才回退到固定文案。 */
export const progressLabel = computed(
  () => state.progress.message || PROGRESS_LABELS[state.progress.phase] || DEFAULT_PROGRESS_LABEL,
);

/** 已知总量时给出百分比，否则交给 UI 显示不确定态。 */
export const progressPercent = computed(() => {
  const { received, total } = state.progress;
  if (!(total > 0)) return null;
  return Math.min(100, Math.max(0, Math.round((received / total) * 100)));
});

/** 速率与剩余时间：服务端和浏览器两侧的字节流都用同一套格式。 */
export const progressDetail = computed(() => formatProgressDetail(state.progress));

/** 当前模式支持的 Mod；加载完成前按 standard 展示，避免控件闪烁。 */
export const modTokens = computed(() => MOD_OPTIONS[state.modeKey] ?? []);
/** DA 参数只在勾选 DA 后展开：默认隐藏，避免占掉抽屉里一大块位置。 */
export const daVisible = computed(() => modTokens.value.includes('DA') && state.mods.includes('DA'));

// 非响应式的内部状态：这些值每帧都会变，放进 reactive 只会带来无意义的依赖追踪。
let session = null;
let backgroundBitmap = null;
let canvasEl = null;
let viewportEl = null;
let viewportObserver = null;
let wasmReady = null;
let animation = 0;
let absoluteStart = 0;
let beatmapSpeed = 1;
let targetFps = 60;
let lastTick = 0;
let lastRenderTime = 0;
let hasRenderedFrame = false;
let audioEnded = false;
/**
 * 音频时钟状态。
 *
 * `lastSeekedMs` 记录「已经落实到音频元素上的进度」：同步到同一个进度时不必再次
 * seek。移动端 seek 会重新缓冲，重复 seek 就是音画不同步的根源。
 */
let audioSeekPending = false;
let lastSeekedMs = Number.NaN;
let audioPlayWhenSeeked = false;
/**
 * 正在重建会话或改画布尺寸。
 *
 * 这些操作在移动端会阻塞主线程几百毫秒到数秒（GPU 资源重建 + 重新上传背景），
 * 期间画面时钟不能继续按墙钟往前跑，否则操作结束后画面已经跑到音频前面。
 */
let renderPending = false;
/**
 * 已经在日志里写过一次播放被拦，避免 tick 里的重试把日志刷屏。
 *
 * 它只抑制日志，不参与静音角标的状态判定。
 */
let audioBlockLogged = false;
let pendingAudioRequest = null;
/**
 * 用户手势恢复音频成功的时刻（performance.now()），0 表示本次手势没有恢复过。
 *
 * 恢复发生在 pointerdown 捕获阶段，而紧随其后的 click 会走到「点击画面 = 播放 /
 * 暂停」上：如果用户只是想把声音打开，那一次点击不该顺带把画面也暂停掉，所以
 * 点击处理会先消费掉这个标记。
 */
let audioResumedAt = 0;
let lastAudioStartAttempt = 0;
let audioObjectUrl = null;
let modSwitching = false;
let appliedMods = [];
/**
 * 加载令牌：每次 `loadPreview()` 递增。
 *
 * 音频与背景在进入播放页之后才继续下载，期间用户可能又加载了别的谱面、或者退回
 * 加载页；每个 await 之后都要用令牌确认自己还是「当前这次加载」，否则旧请求回来
 * 会把新会话的状态覆盖掉。
 */
let loadToken = 0;
/** 当前加载的取消句柄：切谱面或退回加载页时中止还在飞的请求。 */
let loadAbort = null;
/**
 * 浏览器正在收的媒体字节数，按资源名（audio / background）分别记录。
 *
 * 两个资源是并行下的，各自的阶段推进由它们分别汇报；聚合出「一共收了多少」才能
 * 让进度条显示成一个整体，而不是在两个数字之间来回跳。
 */
const mediaProgress = new Map();
/** 最近一次进度是前端自己推进的还是服务端轮询来的；用于避免旧阶段盖掉新阶段。 */
let lastProgressSource = 'local';
/** 当前阶段的开始时刻与各阶段累计耗时，结束后写进播放日志。 */
let stageActive = 'idle';
let stageStartedAt = 0;
const stageDurations = new Map();
// 长按右键时的临时倍速；为 null 表示用界面里选的倍速。
let speedOverride = null;
const arrowTimers = { left: 0, right: 0 };
let rewindTimer = 0;
let rewindResume = false;
// 进度条的计时器与「按住不放」的原因集合（指针悬停、焦点在里面）。
let timelineTimer = 0;
const timelinePins = new Set();

// 音频元素不进 DOM：它只负责出声，画面完全由 WASM 绘制。
const audio = new Audio();
audio.preload = 'auto';

// ---------------------------------------------------------------------------
// 工具
// ---------------------------------------------------------------------------

export const formatTime = (milliseconds) => {
  const seconds = Math.max(0, Math.floor(milliseconds / 1000));
  const minutes = Math.floor(seconds / 60);
  return `${String(minutes).padStart(2, '0')}:${String(seconds % 60).padStart(2, '0')}`;
};

const errorText = (error) => {
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === 'string') return error;
  try { return JSON.stringify(error); } catch (_) { return String(error); }
};

const stamp = () => new Date().toLocaleTimeString();

const logLoad = (message) => { state.loadLogs.push(`${stamp()} ${errorText(message)}`); };
const logPlay = (message) => { state.playLogs.push(`${stamp()} ${errorText(message)}`); };

const playbackRate = () => (speedOverride ?? state.speed) * beatmapSpeed;
const absoluteTime = () => absoluteStart + state.position;

// ---------------------------------------------------------------------------
// 加载进度的格式化与速率采样
// ---------------------------------------------------------------------------

/** 字节数：1 MiB 以上用 MiB，否则用 KiB，避免出现「0.0 MiB」。 */
const formatBytes = (bytes) => {
  if (!(bytes > 0)) return '0 KiB';
  const mib = bytes / 1024 / 1024;
  if (mib >= 1) return `${mib.toFixed(1)} MiB`;
  return `${Math.max(1, Math.round(bytes / 1024))} KiB`;
};

const formatRate = (bytesPerSecond) => `${formatBytes(bytesPerSecond)}/s`;

/** 剩余时间：只给量级，避免进度条上的数字乱跳。 */
const formatEta = (seconds) => {
  if (!Number.isFinite(seconds) || seconds < 0) return '';
  if (seconds < 1) return '不到 1 秒';
  if (seconds < 60) return `约 ${Math.round(seconds)} 秒`;
  return `约 ${Math.ceil(seconds / 60)} 分钟`;
};

/**
 * 拼出「已传 / 总量 · 速度 · 剩余」这一段。
 *
 * 服务端阶段与浏览器下载阶段共用：两者都只提供字节数与总量，速度由采样算出来。
 * 总量未知时只报已传字节数，不编造百分比。
 */
function formatProgressDetail({ received, total, speed, eta }) {
  const parts = [];
  if (total > 0) parts.push(`${formatBytes(received)} / ${formatBytes(total)}`);
  else if (received > 0) parts.push(`已接收 ${formatBytes(received)}`);
  if (speed > 0) parts.push(formatRate(speed));
  const remaining = formatEta(eta);
  if (remaining && total > received) parts.push(remaining);
  return parts.join(' · ');
}

/** 速率采样的窗口长度；太短会让数字乱跳，太长则反应迟钝。 */
const RATE_WINDOW = 2500;
/** 低于这个字节数的变化不更新速率，避免 0 字节时把速度显示成 0。 */
const RATE_MIN_BYTES = 64 * 1024;

/** 滑动窗口采样器：同一阶段内用最近几秒的增量算速度与剩余时间。 */
function createRateSampler() {
  let phase = '';
  let samples = [];
  return {
    /** 推入一次样本，返回当前速率（字节/秒）与剩余秒数。 */
    push({ phase: nextPhase, received, total }) {
      const now = performance.now();
      if (nextPhase !== phase) {
        phase = nextPhase;
        samples = [];
      }
      samples.push({ at: now, received });
      while (samples.length > 1 && now - samples[0].at > RATE_WINDOW) samples.shift();
      const oldest = samples[0];
      const elapsed = now - oldest.at;
      if (elapsed < 250 || received - oldest.received < RATE_MIN_BYTES) {
        return { speed: 0, eta: null };
      }
      const speed = ((received - oldest.received) * 1000) / elapsed;
      const remaining = total > received ? (total - received) / speed : 0;
      return { speed, eta: Number.isFinite(remaining) ? remaining : null };
    },
    reset() {
      phase = '';
      samples = [];
    },
  };
}

const loadRateSampler = createRateSampler();

/** 重置整个加载进度（每次开始加载或退回加载页时调用）。 */
function resetProgress(phase = 'idle') {
  loadRateSampler.reset();
  mediaProgress.clear();
  stageActive = phase;
  stageStartedAt = performance.now();
  stageDurations.clear();
  lastProgressSource = 'local';
  state.progress = { phase, received: 0, total: 0, message: '', speed: 0, eta: null };
}

/** 把当前阶段的已用时间结账，并开启新阶段。 */
function switchStage(next) {
  if (next === stageActive) return;
  stageDurations.set(stageActive, (stageDurations.get(stageActive) ?? 0) + (performance.now() - stageStartedAt));
  stageActive = next;
  stageStartedAt = performance.now();
}

/** 结束时把最后一段未结账的时间也算进去。 */
function stageTotals() {
  const totals = new Map(stageDurations);
  totals.set(stageActive, (totals.get(stageActive) ?? 0) + (performance.now() - stageStartedAt));
  return totals;
}

/**
 * 记录一个阶段的进度；速度与剩余时间由采样器补齐。
 *
 * 只有「前端自己推进」的阶段才带 source='local'：服务端轮询回来的旧阶段不能盖掉
 * 它。典型场景是前端已经进了播放页开始下音频（media），而轮询仍在下发更早的
 * transfer——同一份数据在两条链路上流动，谁最新以本地为准。
 */
function reportProgress({ phase, received = 0, total = 0, message = '', source = 'server' }) {
  if (source === 'local') lastProgressSource = 'local';
  switchStage(phase);
  const { speed, eta } = loadRateSampler.push({ phase, received, total });
  state.progress = { phase, received, total, message, speed, eta };
}

/**
 * 把各阶段耗时写进播放日志。
 *
 * 云端部署时「慢」可能来自镜像、服务端解包或本地带宽，把这几个数字摊开才判断得出
 * 该优化哪一段。
 */
function logStageBreakdown() {
  const entries = [...stageTotals().entries()].filter(([phase]) => PROGRESS_LABELS[phase]);
  if (entries.length === 0) return;
  const text = entries
    .map(([phase, ms]) => `${PROGRESS_LABELS[phase]} ${(ms / 1000).toFixed(1)}s`)
    .join(' · ');
  logPlay(`耗时：${text}`);
  stageDurations.clear();
}

/**
 * 汇报浏览器侧某个资源的接收进度。
 *
 * 两个资源并行下载，谁先报都行：这里把它们的字节数加在一起，按 media 阶段上报。
 */
function reportMediaProgress({ name, received, total, done = false }) {
  if (done) mediaProgress.delete(name);
  else mediaProgress.set(name, { received, total });
  let receivedTotal = 0;
  let bytesTotal = 0;
  for (const entry of mediaProgress.values()) {
    receivedTotal += entry.received;
    bytesTotal += entry.total;
  }
  if (mediaProgress.size === 0) {
    // 两个资源都收完了：数量已经确定，交给调用方推进到 ready。
    reportProgress({ phase: 'media', received: bytesTotal, total: bytesTotal, message: '音频与背景就绪', source: 'local' });
    return;
  }
  reportProgress({ phase: 'media', received: receivedTotal, total: bytesTotal, message: '下载音频与背景', source: 'local' });
}

// ---------------------------------------------------------------------------
// 进度条的显示与隐藏
// ---------------------------------------------------------------------------

const timelinePinned = () => timelinePins.size > 0;

/**
 * 显示进度条并重新计时。
 *
 * 隐藏只改透明度，进度条所在的盒子始终留在布局里，所以画面不会上下位移。
 */
export function showTimeline() {
  window.clearTimeout(timelineTimer);
  timelineTimer = 0;
  state.timelineVisible = true;
  scheduleTimelineHide();
}

/** 指针停在进度条上或焦点在进度条里时保持可见，离开后再重新计时。 */
export function pinTimeline(reason, active) {
  if (active) timelinePins.add(reason);
  else timelinePins.delete(reason);
  if (active) {
    window.clearTimeout(timelineTimer);
    timelineTimer = 0;
    state.timelineVisible = true;
    return;
  }
  scheduleTimelineHide();
}

function scheduleTimelineHide() {
  window.clearTimeout(timelineTimer);
  if (timelinePinned()) return;
  timelineTimer = window.setTimeout(() => {
    timelineTimer = 0;
    if (timelinePinned()) return;
    state.timelineVisible = false;
  }, TIMELINE_HIDE_DELAY);
}

// ---------------------------------------------------------------------------
// WASM 会话与资源
// ---------------------------------------------------------------------------

async function loadWasm() {
  if (!wasmReady) {
    // 失败时清空缓存，下一次点击「加载预览」还能重新尝试。
    wasmReady = import(/* @vite-ignore */ WASM_URL)
      .then(async (module) => {
        await module.default();
        return module;
      })
      .catch((error) => {
        wasmReady = null;
        throw error;
      });
  }
  return wasmReady;
}

async function fetchBeatmap(value, { signal } = {}) {
  const response = await fetch(`/resource/beatmap?bid=${encodeURIComponent(value)}`, { signal });
  if (!response.ok) throw new Error(await response.text());
  return new Uint8Array(await response.arrayBuffer());
}

/**
 * 读取谱面内部信息。
 *
 * WASM 只按传入的 `.osu` 字节解析（它没有网络能力），`bid` → 字节这一步由后端的
 * `/resource/beatmap` 完成。失败只记日志，不影响预览本身。
 */
function readBeatmapInfo(wasm, bytes) {
  try {
    return wasm.beatmapInfo(bytes);
  } catch (error) {
    logPlay(`谱面信息解析失败：${errorText(error)}`);
    return null;
  }
}

// ---------------------------------------------------------------------------
// 加载进度
// ---------------------------------------------------------------------------

let progressTimer = 0;

/**
 * 合并服务端上报的进度。
 *
 * 服务端只知道「自己下到哪了 / 解包到哪了 / 开始发包了」，浏览器接收字节的进度由
 * 前端自己统计（media 阶段），因此这里不能反过来覆盖掉本地阶段：谁的信息更靠后
 * 就以谁为准，`PHASE_ORDER` 就是这个先后关系。
 */
function applyProgress(payload) {
  const phase = typeof payload?.phase === 'string' ? payload.phase : 'idle';
  if (phase === 'idle' || phase === 'ready' || phase === 'error') {
    // ready 由前端在真正进入播放页时自己上报，避免服务端提前把进度条推到终点。
    if (phase === 'error') reportProgress({ phase: 'error', message: String(payload?.message ?? '') });
    return;
  }
  // 前端已经自己推进过阶段（例如开始下载音频）时，服务端轮询回来的旧阶段一律忽略。
  if (lastProgressSource === 'local') return;
  if (phaseOrder(phase) < phaseOrder(state.progress.phase)) return;
  reportProgress({
    phase,
    received: Math.max(0, Number(payload?.received) || 0),
    total: Math.max(0, Number(payload?.total) || 0),
    message: typeof payload?.message === 'string' ? payload.message : '',
  });
}

/** 阶段先后顺序：越靠后表示加载越接近完成。 */
const PHASE_ORDER = ['idle', 'osu', 'osz', 'extract', 'transfer', 'media', 'ready', 'error'];
const phaseOrder = (phase) => {
  const index = PHASE_ORDER.indexOf(phase);
  return index < 0 ? 0 : index;
};

function startProgressPolling(bid) {
  stopProgressPolling();
  const poll = () => {
    fetch(`/resource/progress?bid=${encodeURIComponent(bid)}`)
      .then((response) => (response.ok ? response.json() : null))
      .then((payload) => { if (payload) applyProgress(payload); })
      // 轮询只是锦上添花，失败不能影响真正的加载流程。
      .catch(() => {});
  };
  poll();
  progressTimer = window.setInterval(poll, PROGRESS_INTERVAL);
}

function stopProgressPolling() {
  window.clearInterval(progressTimer);
  progressTimer = 0;
}

// ---------------------------------------------------------------------------
// 深链 ?bid=<BID>[&convert=<模式>]
// ---------------------------------------------------------------------------

/** 解析 URL 参数；bid 不是纯数字时返回 null，表示没有可用的深链。 */
export function readDeepLink(search = window.location.search) {
  const params = new URLSearchParams(search);
  const bid = (params.get('bid') ?? '').trim();
  if (!/^\d+$/.test(bid)) return null;
  const convert = (params.get('convert') ?? '').trim().toLowerCase();
  return { bid, convert: CONVERT_MODES.includes(convert) ? convert : '' };
}

/** 把当前 bid 写回地址栏，刷新或分享都能直接回到同一个预览。 */
function syncUrl() {
  const params = new URLSearchParams();
  if (state.bid) params.set('bid', state.bid);
  if (state.convert) params.set('convert', state.convert);
  const query = params.toString();
  window.history.replaceState(null, '', query ? `${window.location.pathname}?${query}` : window.location.pathname);
}

/**
 * 带字节进度的抓取。
 *
 * `fetch` 本身不给进度，只能读响应体流自己累计；`Content-Length` 缺失时总量为 0，
 * UI 会退化成不确定态，而不是编一个假百分比出来。
 */
async function fetchWithProgress(url, { signal, onProgress } = {}) {
  const response = await fetch(url, { signal });
  if (!response.ok) throw new Error(await response.text());
  const total = Number(response.headers.get('content-length')) || 0;
  if (!response.body) {
    const buffer = new Uint8Array(await response.arrayBuffer());
    onProgress?.({ received: buffer.byteLength, total });
    return buffer;
  }
  const reader = response.body.getReader();
  const chunks = [];
  let received = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    chunks.push(value);
    received += value.byteLength;
    onProgress?.({ received, total });
  }
  const merged = new Uint8Array(received);
  let offset = 0;
  for (const chunk of chunks) {
    merged.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return merged;
}

async function loadBackground(value, { signal } = {}) {
  try {
    const bytes = await fetchWithProgress(`/resource/background?bid=${encodeURIComponent(value)}`, {
      signal,
      onProgress: ({ received, total }) => reportMediaProgress({ name: 'background', received, total }),
    });
    const bitmap = await createImageBitmap(new Blob([bytes]));
    if (backgroundBitmap) backgroundBitmap.close();
    backgroundBitmap = bitmap;
    paintBackground();
  } finally {
    // 失败也要注销，否则进度条会一直等着一个永远不会到的资源。
    reportMediaProgress({ name: 'background', done: true });
  }
}

/** 把背景按当前画布尺寸解码、暗化后交给 WASM 合成。 */
function paintBackground() {
  if (!session || !backgroundBitmap) return;
  const decoder = document.createElement('canvas');
  decoder.width = session.width();
  decoder.height = session.height();
  const decoderContext = decoder.getContext('2d', { willReadFrequently: true });
  decoderContext.drawImage(backgroundBitmap, 0, 0, decoder.width, decoder.height);
  const rgba = decoderContext.getImageData(0, 0, decoder.width, decoder.height).data;
  const brightness = 1 - BACKGROUND_DIM;
  for (let index = 0; index < rgba.length; index += 4) {
    rgba[index] = Math.round(rgba[index] * brightness);
    rgba[index + 1] = Math.round(rgba[index + 1] * brightness);
    rgba[index + 2] = Math.round(rgba[index + 2] * brightness);
  }
  session.set_background_rgba(decoder.width, decoder.height, rgba);
}

/**
 * 下载音频并挂到音频元素上。
 *
 * 用 blob URL 而不是直接给 `audio.src` 指向接口：blob 已经在你机器上，
 * 之后反复 seek 不会每次再向服务器要一遍几 MiB（尤其是播放页的倒带）。
 * 代价是要先下完才能播，所以调用方把它放在关键路径之外。
 */
async function loadAudioBlob(value, { signal } = {}) {
  let bytes;
  try {
    bytes = await fetchWithProgress(`/resource/audio?bid=${encodeURIComponent(value)}`, {
      signal,
      onProgress: ({ received, total }) => reportMediaProgress({ name: 'audio', received, total }),
    });
  } finally {
    reportMediaProgress({ name: 'audio', done: true });
  }
  if (!bytes.byteLength) throw new Error('音频资源为空');
  const blob = new Blob([bytes]);
  releaseAudioUrl();
  audioObjectUrl = URL.createObjectURL(blob);
  audio.src = audioObjectUrl;
  resetAudioClock();
  audio.load();
  await new Promise((resolve, reject) => {
    const timer = window.setTimeout(() => {
      cleanup();
      reject(new Error('音频元数据加载超时'));
    }, AUDIO_METADATA_TIMEOUT);
    const ready = () => { cleanup(); resolve(); };
    const failed = () => { cleanup(); reject(new Error(audio.error?.message || '音频无法解码或播放')); };
    const cleanup = () => {
      window.clearTimeout(timer);
      audio.removeEventListener('loadedmetadata', ready);
      audio.removeEventListener('canplay', ready);
      audio.removeEventListener('error', failed);
    };
    audio.addEventListener('loadedmetadata', ready, { once: true });
    audio.addEventListener('canplay', ready, { once: true });
    audio.addEventListener('error', failed, { once: true });
  });
}

function releaseAudioUrl() {
  if (!audioObjectUrl) return;
  URL.revokeObjectURL(audioObjectUrl);
  audioObjectUrl = null;
}

/** 会话重建（加载、切 Mod、切分辨率）后同步时长与时间轴换算基准。 */
function applySessionMetrics() {
  state.duration = Math.max(1, session.duration_ms_number());
  absoluteStart = session.absolute_start_ms_number();
  beatmapSpeed = session.beatmap_speed_number();
}

// ---------------------------------------------------------------------------
// 画布尺寸
// ---------------------------------------------------------------------------

// canvas 的 width/height 属性是 WASM/GPU 的像素尺寸；CSS 显示尺寸由这里按
// viewport 内容区做 contain 计算，避免浏览器只按宽度缩放导致底部被裁掉。
export function fitCanvasToViewport() {
  if (!session || !canvasEl || !viewportEl) return;
  const style = getComputedStyle(viewportEl);
  const availableWidth = viewportEl.clientWidth
    - parseFloat(style.paddingLeft)
    - parseFloat(style.paddingRight);
  const availableHeight = viewportEl.clientHeight
    - parseFloat(style.paddingTop)
    - parseFloat(style.paddingBottom);
  if (availableWidth <= 0 || availableHeight <= 0) return;
  const sourceWidth = session.width();
  const sourceHeight = session.height();
  if (!(sourceWidth > 0 && sourceHeight > 0)) return;
  const scale = Math.min(availableWidth / sourceWidth, availableHeight / sourceHeight);
  canvasEl.style.width = `${Math.max(1, Math.floor(sourceWidth * scale))}px`;
  canvasEl.style.height = `${Math.max(1, Math.floor(sourceHeight * scale))}px`;
}

/** 播放页挂载时登记画布与容器；窗口尺寸、侧栏变化都靠观察容器重新 contain。 */
export function attachStage({ viewport, canvas }) {
  viewportEl = viewport;
  canvasEl = canvas;
  if (window.ResizeObserver) {
    viewportObserver?.disconnect();
    viewportObserver = new ResizeObserver(() => fitCanvasToViewport());
    viewportObserver.observe(viewportEl);
  } else {
    window.addEventListener('resize', fitCanvasToViewport);
  }
  window.addEventListener('orientationchange', fitCanvasToViewport);
  if (session) {
    canvasEl.width = session.width();
    canvasEl.height = session.height();
    fitCanvasToViewport();
  }
}

// ---------------------------------------------------------------------------
// 渲染与播放
// ---------------------------------------------------------------------------

function render() {
  if (!session) return;
  try {
    session.render_number(absoluteTime());
    state.rendered = true;
    state.renderError = '';
  } catch (error) {
    playState(false);
    state.renderError = '渲染失败，请查看日志';
    logPlay(`渲染失败：${errorText(error)}`);
  }
}

/**
 * 把进度换算成音频时间。
 *
 * 这里是移动端音画不同步的关键：`currentTime` 的赋值是异步 seek，手机上真正落地
 * 可能要好几秒，期间音频还在从旧位置出声。所以
 *   1. 只在「确实需要挪动」时才 seek（重复同步同一进度不做任何事）；
 *   2. seek 期间冻结画面时钟（audioSeekPending），让画面等音频而不是反过来；
 *   3. seek 完成后由 tick 里的对齐逻辑接管（见 applyAudioClock）。
 *
 * 目标进度统一记在 `lastSeekedMs` 上（见 seekAudioTo）：还没拿到元数据时先不下发，
 * 等 loadedmetadata 或 tick 再补——这样不会出现「先记着待办、又被别的路径清掉」的状态。
 */
function syncAudioToPosition({ resume = false } = {}) {
  const time = absoluteTime();
  audioEnded = false;
  if (time < 0) {
    // 谱面开始前的静音段：直接停在 0，等真正进入正片再开始播。
    audio.pause();
    if (Number.isFinite(audio.duration)) {
      audio.currentTime = 0;
      lastSeekedMs = absoluteStart;
    }
    return;
  }
  seekAudioTo(Math.min(time, Number.isFinite(audio.duration) ? audio.duration * 1000 : time), { resume });
}

/** 目标进度是否还没有落实到音频元素上（元数据没到，或 seek 还在飞）。 */
function audioOutOfSync() {
  if (!Number.isFinite(lastSeekedMs)) return false;
  if (audioSeekPending) return false;
  return Math.abs(lastSeekedMs - absoluteTime()) > AUDIO_RESYNC_THRESHOLD;
}

/**
 * 请求音频 seek 到指定进度。
 *
 * 用 seeked 事件而不是「设完就当完成」：手机上一次 seek 会重新缓冲，只有事件到了
 * 才知道新位置真的生效；期间画面冻结，结束后按需要对上播放状态。
 *
 * 已经在目标附近时什么都不做——播放、恢复、切 mod 都会调用到这里，真去 seek 的话
 * 手机上每按一次「启用/禁用」都会卡一下声音。
 */
function seekAudioTo(targetMs, { resume = false } = {}) {
  const durationMs = Number.isFinite(audio.duration) ? audio.duration * 1000 : targetMs;
  const clamped = Math.min(Math.max(0, targetMs), durationMs);
  if (Number.isFinite(lastSeekedMs) && Math.abs(lastSeekedMs - clamped) <= AUDIO_RESYNC_THRESHOLD) return;
  // 已经有 seek 在飞：只记下最新目标，等它落地后再补一次。
  // 倒带时每 120ms 就跳一步，直接丢弃这些请求会让音频停在旧位置。
  lastSeekedMs = clamped;
  if (audioSeekPending) return;
  // 元数据还没到就先不下发：记在 lastSeekedMs 上，loadedmetadata 或 tick 会补。
  if (!Number.isFinite(audio.duration)) return;
  audioSeekPending = true;
  // 先停住再改 currentTime：手机上 seek 要几百毫秒才落地，期间如果继续出声，
  // 听到的还是旧位置，而画面已经冻在新位置——那就是「声音和画面对不上」。
  audio.pause();
  audio.currentTime = clamped / 1000;
  return waitForAudioSeek().then(() => {
    audioSeekPending = false;
    // seek 期间被暂停的音频要恢复，但只在「本来就在播」的前提下。
    const shouldPlay = (resume || audioPlayWhenSeeked) && state.playing;
    audioPlayWhenSeeked = false;
    if (shouldPlay) startAudio();
    // 等待期间又来了新的目标（连续拖动进度条、倒带）：补一次 seek。
    if (Math.abs(lastSeekedMs - clamped) > AUDIO_RESYNC_THRESHOLD) {
      void seekAudioTo(lastSeekedMs, { resume: shouldPlay });
    }
  });
}

/** 等到 seek 真正落地（或超时放行）。 */
function waitForAudioSeek(timeout = AUDIO_SEEK_TIMEOUT) {
  return new Promise((resolve) => {
    const done = () => {
      window.clearTimeout(timer);
      audio.removeEventListener('seeked', done);
      resolve();
    };
    const timer = window.setTimeout(done, timeout);
    audio.addEventListener('seeked', done, { once: true });
  });
}

/** 重新加载音频资源后，之前记录的 seek 位置失效。 */
function resetAudioClock() {
  lastSeekedMs = Number.NaN;
  audioSeekPending = false;
  audioPlayWhenSeeked = false;
}

/**
 * 用音频时钟校正画面进度。
 *
 * 与音频差得不多时让画面贴住音频；差得太多说明两边已经走散，宁可重新 seek 一次
 * 也不要长期错位——但重新 seek 会冻结画面，所以小范围漂移交给音频自己追。
 */
function applyAudioClock() {
  const audioPosition = audio.currentTime * 1000 - absoluteStart;
  if (!Number.isFinite(audioPosition)) return;
  const drift = audioPosition - state.position;
  if (Math.abs(drift) <= AUDIO_RESYNC_THRESHOLD) {
    state.position = Math.min(state.duration, Math.max(0, audioPosition));
    return;
  }
  // 差距不大时保持当前进度、等音频追上来，避免为了几十毫秒反复 seek。
  if (Math.abs(drift) < AUDIO_RESYNC_MAX_WAIT) return;
  // 这里要发起新的 seek，画面会一直冻到 seeked 事件到达；期间不再重复触发，
  // 否则一个 seek 还没落地就又发一个，手机上一次也完不成。
  if (!audioSeekPending) seekAudioTo(absoluteTime(), { resume: true });
}

/** 记录音频「已经出声」：清掉静音状态与提示，之后失败也不会再重复写日志。 */
function markAudioAudible() {
  state.audioBlocked = false;
  audioBlockLogged = false;
}

/**
 * 请求播放音频。
 *
 * 播到结尾、暂停或 seek 都会让上一次尚未落地的 play() 被 pause() 打断，
 * 此时浏览器会抛 AbortError。这属于正常流程，只有真正播不出来才写日志。
 *
 * 返回值告诉手势恢复逻辑这次请求的结果：`{ rejected, blocked }`，
 * `blocked` 专指被自动播放策略拒绝。
 */
function requestAudioPlay() {
  if (!audio.src) return Promise.resolve({ rejected: false, blocked: false });
  // 同一时刻只留一个未落地的 play()：并行请求只会互相打断并抛 AbortError。
  if (pendingAudioRequest) return pendingAudioRequest;
  lastAudioStartAttempt = performance.now();
  const request = audio.play();
  if (!request) return Promise.resolve({ rejected: false, blocked: false });
  const guarded = request.then(
    () => ({ rejected: false, blocked: false }),
    (error) => {
      // 暂停、seek 或新的 seek 打断 play() 都属于正常流程：只有画面还在播时才算真正的失败。
      if (!state.playing || error?.name === 'AbortError' || audioSeekPending) {
        return { rejected: false, blocked: false };
      }
      const blocked = error?.name === 'NotAllowedError';
      if (blocked) {
        // 自动播放策略一旦拒绝就会一直拒绝，所以只写一次日志并提示用户点一下。
        state.audioBlocked = true;
        if (!audioBlockLogged) {
          audioBlockLogged = true;
          logPlay('浏览器拦截了自动播放，点一下画面或按任意键即可播放声音。');
        }
        return { rejected: true, blocked: true };
      }
      if (!audioBlockLogged) {
        audioBlockLogged = true;
        logPlay(`音频播放失败，继续播放画面：${errorText(error)}`);
      }
      return { rejected: true, blocked: false };
    },
  );
  pendingAudioRequest = guarded.finally(() => { pendingAudioRequest = null; });
  return pendingAudioRequest;
}

/** 画面时间可用时请求播放音频；seek 到 0 之前或已经播完则不必请求。 */
function startAudio() {
  if (absoluteTime() < 0 || audioEnded) return;
  requestAudioPlay();
}

/**
 * 在用户手势里恢复音频。
 *
 * 自动播放被拦下后，只有用户激活（指针 / 触摸 / 键盘）里的 play() 才会被放行，
 * 定时器里的重试永远会被拒。手势恢复成功时记下时刻，让紧随其后的 click 不要
 * 顺手把画面暂停掉。
 */
function resumeAudioFromGesture() {
  if (!state.playing) return Promise.resolve(false);
  if (!state.audioBlocked && !audio.paused && !audio.ended) return Promise.resolve(false);
  return requestAudioPlay().then(({ rejected }) => {
    // play() 被拒绝也可能只是又被 pause() 打断，真正算数的是音频是否已经在响。
    if (rejected || audio.paused) return false;
    markAudioAudible();
    audioResumedAt = performance.now();
    return true;
  });
}

/**
 * 首次真实输入（指针 / 触摸 / 键盘）时恢复音频。
 *
 * 用捕获阶段监听：手势必须在事件处理的最前面把 play() 发出去，才能算「由用户
 * 激活触发的播放」，后面的 click 也来不及把这次激活用掉。
 */
function handleUserGesture() {
  if (!state.playing || audio.ended) return;
  if (!state.audioBlocked && !audio.paused) return;
  resumeAudioFromGesture().then((resumed) => {
    // 恢复成功就解除监听：音频回到暂停只可能是用户自己按的暂停，
    // 那时再自动重播会对着干。
    if (resumed) removeGestureHints();
  });
}

function addGestureHints() {
  for (const type of GESTURE_EVENTS) window.addEventListener(type, handleUserGesture, { capture: true, passive: true });
}

function removeGestureHints() {
  for (const type of GESTURE_EVENTS) window.removeEventListener(type, handleUserGesture, { capture: true });
}

/**
 * 把画面进度对齐到音频当前所在的位置。
 *
 * 暂停时必须做这一步：`state.position` 是上一次 rAF 时更新的，而 `audio.pause()`
 * 是立即生效的，两者最多差一个渲染帧；如果暂停时不对齐，恢复播放时时钟一交回
 * 音频，画面就会往回跳一帧（就是「暂停再播放跳帧」）。
 *
 * 只用「离当前进度 80ms 以内」的读数：seek 途中或两边已经错位时，音频的瞬时值
 * 不可信（比如刚拖完进度条、或手势恢复后又被这次点击暂停），宁可不动。
 */
function alignPositionToAudio() {
  if (audioSeekPending || !Number.isFinite(audio.duration)) return;
  const audioPosition = audio.currentTime * 1000 - absoluteStart;
  if (!Number.isFinite(audioPosition) || Math.abs(audioPosition - state.position) > PAUSE_ALIGN_TOLERANCE) return;
  state.position = Math.min(state.duration, Math.max(0, audioPosition));
}

function playState(next) {
  state.playing = next;
  lastTick = 0;
  lastRenderTime = 0;
  hasRenderedFrame = false;
  cancelAnimationFrame(animation);
  audio.playbackRate = playbackRate();
  if (next) {
    syncAudioToPosition();
    // 恢复播放时也对齐一次：暂停期间画面可能因为标签页/主线程节流而落后于音频，
    // 不对齐的话恢复瞬间会先跳一下再被音频时钟拉回来。
    alignPositionToAudio();
    startAudio();
    animation = requestAnimationFrame(tick);
  } else {
    // 先把进度钉在音频当下所在的位置再暂停，恢复时才不会跳回上一帧。
    const before = state.position;
    alignPositionToAudio();
    audio.pause();
    // 定格的那一帧要和音频停下的位置一致，否则暂停画面本身就是旧的一帧。
    if (before !== state.position) render();
  }
}

/** 播到结尾：停在最后一帧，音频停住，等用户再点播放。 */
function finishPlayback() {
  state.position = state.duration;
  if (state.playing) playState(false);
}

/** 从结尾重新开始时先把时钟拨回开头，否则第一帧又会判定结束。 */
function restartPlayback() {
  state.position = 0;
  audioEnded = false;
  syncAudioToPosition();
  render();
}

function tick(now) {
  if (!state.playing) return;
  if (!lastTick) lastTick = now;
  const delta = now - lastTick;
  lastTick = now;

  // 音频 seek 还没落地（移动端要几秒）、还没拿到元数据、或正在重建渲染资源：
  // 画面停在原地等，否则等这一下结束时两边已经错开，正好是「跳进度条 / 切 mod /
  // 切帧率之后不同步」的来源。
  if (audioSeekPending || audioOutOfSync() || renderPending) {
    state.position = Math.min(state.duration, Math.max(0, state.position));
  } else if (absoluteTime() < 0 || audio.paused || audioEnded) {
    state.position = Math.min(state.duration, state.position + delta * playbackRate());
    // 音频还在缓冲或等待用户激活时，画面继续走并定期重试。
    //
    // 自动播放被拦下时这里的重试一定还是会被拒（定时器里没有用户激活），真正的
    // 恢复靠 handleUserGesture，这里只负责缓冲结束和 seek 之后的自动续播。
    if (absoluteTime() >= 0 && audio.paused && !audioEnded
      && now - lastAudioStartAttempt >= AUDIO_RETRY_INTERVAL) {
      startAudio();
    }
  } else {
    applyAudioClock();
  }

  // 始终按显示刷新率推进时钟，但只按所选帧率提交渲染。用目标帧间隔
  // 累加而不是直接用 `now` 覆盖，避免 144Hz 等显示器上 60FPS 被降到 48FPS。
  const frameInterval = 1000 / targetFps;
  if (!hasRenderedFrame) {
    hasRenderedFrame = true;
    lastRenderTime = now;
    render();
  } else if (now - lastRenderTime >= frameInterval) {
    lastRenderTime += frameInterval * Math.floor((now - lastRenderTime) / frameInterval);
    render();
  }

  if (!state.playing) return;
  if (state.position >= state.duration) {
    finishPlayback();
    return;
  }
  animation = requestAnimationFrame(tick);
}

// ---------------------------------------------------------------------------
// 对外的操作
// ---------------------------------------------------------------------------

/**
 * 加载一份谱面并进入播放页。
 *
 * 关键路径只保留「取谱面 → 建会话 → 拿背景」：背景是画面的一部分，等它是值得的；
 * 音频通常几十 MiB，让它和播放并行下载——想听声音的用户等这几秒，想先看画面的
 * 用户不必等。每个 await 之后都用 loadToken 确认自己还是当前这次加载。
 */
export async function loadPreview() {
  if (state.loading) return;
  state.loadLogs = [];
  state.loading = true;
  state.info = null;
  cancelArrowHold();
  // 上一次加载可能还在后台拉音频/背景：先取消，避免两套请求互相覆盖状态。
  loadAbort?.abort();
  const controller = new AbortController();
  loadAbort = controller;
  const token = ++loadToken;
  const current = () => token === loadToken;
  resetProgress('osu');
  state.preparingMedia = false;
  try {
    const bid = state.bid.trim();
    if (!bid) throw new Error('请输入谱面 BID');
    if (!canvasEl) throw new Error('画布尚未就绪，请刷新页面重试');
    state.bid = bid;
    const [wasm, bytes] = await Promise.all([
      loadWasm(),
      fetchBeatmap(bid, { signal: controller.signal }),
    ]);
    if (!current()) return;
    state.info = readBeatmapInfo(wasm, bytes);
    const resolution = RESOLUTIONS[state.resolution];
    const options = {
      convert: state.convert || undefined,
      // Mod 只通过播放页的可选控件设置，避免手写 token 与当前模式不匹配。
      mods: [],
      // WASM 会话和 core 的 RealtimeOptions 都使用这组输出宽高。
      width: resolution.width,
      height: resolution.height,
    };
    session = await wasm.WebGpuSession.create(bytes, canvasEl, options);
    if (!current()) return;
    appliedMods = [];
    state.mods = [];
    state.renderError = '';
    state.rendered = false;
    state.audioBlocked = false;
    audioBlockLogged = false;
    audioResumedAt = 0;
    audioEnded = false;
    lastAudioStartAttempt = 0;
    applySessionMetrics();
    state.position = 0;
    state.modeKey = session.mode();
    state.mode = state.modeKey.toUpperCase();
    state.status = '准备音频与背景...';

    canvasEl.width = session.width();
    canvasEl.height = session.height();
    syncAudioToPosition();
    // 会话已经就绪，先渲染一帧：背景还没到时 composer 会用兜底色，画面不是黑屏。
    render();
    // 资源准备通常慢在这里（服务端下载 + 解包），进度阶段由轮询推进到 transfer/media。
    startProgressPolling(bid);
    // 音频与背景一起发起，但只等背景：服务端此时已经把两个文件都解出来了。
    state.preparingMedia = true;
    const background = loadBackground(bid, { signal: controller.signal });
    const audioLoad = loadAudioBlob(bid, { signal: controller.signal })
      .then(() => {
        if (!current()) return;
        state.status = '就绪';
        logStageBreakdown();
      })
      .catch((error) => {
        if (!current()) return;
        state.status = '无音频';
        logPlay(`音频加载失败，将只播放画面：${errorText(error)}`);
        logStageBreakdown();
      })
      .finally(() => { if (current()) state.preparingMedia = false; });
    await background.catch((error) => {
      if (current()) logPlay(`背景加载失败，将使用黑色背景：${errorText(error)}`);
    });
    if (!current()) return;
    state.page = 'play';
    state.sheetOpen = false;
    // 等播放页真正显示出来再量尺寸，否则 clientWidth/Height 还是 0。
    await nextTick();
    fitCanvasToViewport();
    showTimeline();
    render();
    reportProgress({ phase: 'ready', source: 'local' });
    // 加载完成后直接进入播放状态并尝试带声音自动播放：已经在页面上点过「加载
    // 预览」时这次 play() 算用户激活，能直接出声。浏览器若因自动播放策略拒绝，
    // 画面时钟仍由 requestAnimationFrame 继续推进，并由手势监听等待用户激活
    // （首次点击 / 触摸 / 按键）后重试播放声音。音频还没下完时 startAudio() 会
    // 被 play() 的 pending 状态挡住，等元数据事件到达后由 tick 里的重试接手。
    playState(true);
    addGestureHints();
    syncUrl();
    void audioLoad;
  } catch (error) {
    // 取消或已经被新的一次加载取代：错误属于旧请求，不该报给用户。
    if (!current() || error?.name === 'AbortError') return;
    logLoad(error);
  } finally {
    // 被新加载取代或退回了加载页时不能碰 state：那两个入口自己会收拾。
    if (current()) {
      stopProgressPolling();
      state.loading = false;
    }
  }
}

/**
 * 判断这次播放 / 暂停操作是不是「刚刚被用来开启声音的那一次」，是则消费掉。
 *
 * 恢复发生在 pointerdown / keydown 的捕获阶段，紧随其后的 click（点画面）或
 * keydown（空格）会走到播放 / 暂停：用户此刻想要的是有声音，不该顺带把画面暂停。
 */
function consumeResumeGesture() {
  if (!audioResumedAt || performance.now() - audioResumedAt >= GESTURE_RESUME_WINDOW) return false;
  audioResumedAt = 0;
  return true;
}

export function togglePlay() {
  if (!session) return;
  if (state.playing && consumeResumeGesture()) return;
  if (state.playing) {
    playState(false);
    return;
  }
  // 播到结尾后再点播放要从头开始：否则第一帧就会再次判定结束并立刻暂停，
  // 让刚发出的 play() 被 pause() 打断（AbortError: interrupted by a call to pause）。
  if (state.position >= state.duration - 1) restartPlayback();
  playState(true);
}

export function seekTo(nextPosition) {
  if (!session) return;
  const resume = state.playing;
  if (resume) playState(false);
  state.position = Math.min(state.duration, Math.max(0, nextPosition));
  // resume：seek 会让音频元素短暂暂停（见 seekAudioTo），完成后要自己恢复出声，
  // 否则用户跳一次进度条就变成了静音播放。
  syncAudioToPosition({ resume });
  render();
  if (resume) playState(true);
}

export const seekBy = (offset) => seekTo(state.position + offset);

// ---------------------------------------------------------------------------
// 左右方向键：短按在松开时跳转，长按进入快进 / 倒带
// ---------------------------------------------------------------------------

/**
 * 按下方向键。
 *
 * 这里只起一个延时定时器：到点说明是长按，进入快进或倒带；没到点就松开
 * 算一次单击，由 releaseArrowKey 在松开时跳转（避免按下瞬间就跳走）。
 */
export function pressArrowKey(side) {
  if (!session || arrowTimers[side]) return;
  arrowTimers[side] = window.setTimeout(() => {
    arrowTimers[side] = 0;
    if (side === 'right') startFastForward();
    else startRewind();
  }, ARROW_HOLD_DELAY);
}

/** 松开方向键：短按跳转 ±5 秒，长按则结束快进 / 倒带。 */
export function releaseArrowKey(side) {
  if (arrowTimers[side]) {
    window.clearTimeout(arrowTimers[side]);
    arrowTimers[side] = 0;
    seekBy(side === 'right' ? 5000 : -5000);
    return;
  }
  if (side === 'right') stopFastForward();
  else stopRewind();
}

/** 窗口失焦或退回加载页时清掉按键状态，避免卡在快进/倒带。 */
export function cancelArrowHold() {
  for (const side of ['left', 'right']) {
    window.clearTimeout(arrowTimers[side]);
    arrowTimers[side] = 0;
  }
  window.clearInterval(rewindTimer);
  rewindTimer = 0;
  rewindResume = false;
  state.rewinding = false;
  if (!state.fastForwarding) return;
  state.fastForwarding = false;
  speedOverride = null;
  audio.playbackRate = playbackRate();
}

function startFastForward() {
  state.fastForwarding = true;
  speedOverride = FAST_FORWARD_SPEED;
  audio.playbackRate = playbackRate();
}

function stopFastForward() {
  if (!state.fastForwarding) return;
  state.fastForwarding = false;
  speedOverride = null;
  audio.playbackRate = playbackRate();
}

/** 倒带时先停住播放：每步都是一次 seek，继续出声只会变成噪音。 */
function startRewind() {
  state.rewinding = true;
  rewindResume = state.playing;
  if (rewindResume) playState(false);
  rewindTimer = window.setInterval(() => {
    seekTo(state.position - REWIND_STEP);
    showTimeline();
  }, REWIND_INTERVAL);
}

function stopRewind() {
  if (!state.rewinding) return;
  state.rewinding = false;
  window.clearInterval(rewindTimer);
  rewindTimer = 0;
  if (rewindResume && session && state.page === 'play') playState(true);
  rewindResume = false;
}

/**
 * 在改渲染参数期间冻结时钟。
 *
 * 切帧率 / 分辨率都会让移动端主线程卡住几百毫秒（GPU 资源重建、背景重新上传），
 * 而音频不会停下来——不冻结的话操作结束后画面已经跑到音频前面，接着就是一次
 * 补 seek 和一次声音卡顿。所以这里统一「停画面 → 改参数 → 重新对齐时钟 → 恢复」。
 */
function withFrozenClock(mutate) {
  const resume = state.playing;
  if (resume) playState(false);
  renderPending = true;
  try {
    mutate();
  } finally {
    renderPending = false;
    // 恢复播放会让 clock 重新对齐（画布尺寸、mod 都可能改变时间轴）。
    if (resume) playState(true);
  }
}

export function setSpeed(value) {
  state.speed = value;
  audio.playbackRate = playbackRate();
}

export function setFps(value) {
  state.fps = value;
  targetFps = value;
  if (state.page !== 'play') return;
  withFrozenClock(() => {
    lastRenderTime = 0;
    hasRenderedFrame = false;
    render();
  });
}

export function setResolution(key) {
  state.resolution = key;
  if (!session) return;
  const { width, height } = RESOLUTIONS[key];
  if (session.width() === width && session.height() === height) return;
  const previous = Object.keys(RESOLUTIONS).find((candidate) => {
    const size = RESOLUTIONS[candidate];
    return size.width === session.width() && size.height === session.height();
  }) ?? '720';
  withFrozenClock(() => {
    try {
      // session.resize 同时更新 WASM surface 和 core 的合成尺寸，否则场景会
      // 继续按旧宽高绘制并贴在新画布的左上角。
      session.resize(width, height);
      canvasEl.width = width;
      canvasEl.height = height;
      fitCanvasToViewport();
      paintBackground();
      render();
    } catch (error) {
      state.resolution = previous;
      logPlay(`分辨率切换失败：${errorText(error)}`);
    }
  });
}

// DA 需要参数；界面暴露 AR/CS 两个滑杆，勾选 DA 后生成 DAAR<value>CS<value>。
const daNumber = (value) => {
  const number = Number(value);
  return Number.isFinite(number) ? String(number) : '0';
};
export const activeDaToken = () => `DAAR${daNumber(state.daAr)}CS${daNumber(state.daCs)}`;

/**
 * 拖动 DA 滑杆时只更新数值，松手（change）才真正重建会话：
 * 拖动过程中反复 set_mods 会把 WASM 会话卡住。
 */
export function setDaValue(name, value) {
  const number = Number(value);
  if (!Number.isFinite(number)) return;
  if (name === 'ar') state.daAr = number;
  if (name === 'cs') state.daCs = number;
}

/** 松手后自动勾选 DA 并重新应用 Mod。 */
export function commitDa() {
  if (!state.mods.includes('DA')) state.mods.push('DA');
  applyMods();
}

export function toggleMod(token) {
  const index = state.mods.indexOf(token);
  if (index >= 0) state.mods.splice(index, 1);
  else state.mods.push(token);
  applyMods();
}

function applyMods() {
  if (!session || modSwitching) return;
  const requested = state.mods.slice();
  modSwitching = true;
  const submitted = requested
    .filter((token) => token !== 'DA')
    .concat(requested.includes('DA') ? [activeDaToken()] : []);
  try {
    session.set_mods(submitted);
    appliedMods = requested;
    applySessionMetrics();
    audio.playbackRate = playbackRate();
    state.position = Math.min(state.position, state.duration);
    syncAudioToPosition();
    render();
  } catch (error) {
    logPlay(`Mod 切换失败：${errorText(error)}`);
    state.mods = appliedMods.slice();
  } finally {
    modSwitching = false;
  }
}

export function setSheetOpen(open) {
  state.sheetOpen = open;
}

export function backToLoad() {
  playState(false);
  cancelArrowHold();
  cancelAnimationFrame(animation);
  removeGestureHints();
  // 还在后台拉音频/背景的请求要一并中止：否则它们回来时会往已经清空的播放页写状态。
  loadAbort?.abort();
  loadAbort = null;
  loadToken += 1;
  stopProgressPolling();
  resetProgress();
  state.loading = false;
  state.preparingMedia = false;
  audioResumedAt = 0;
  state.audioBlocked = false;
  audioBlockLogged = false;
  releaseAudioUrl();
  audio.removeAttribute('src');
  audio.load();
  if (backgroundBitmap) {
    backgroundBitmap.close();
    backgroundBitmap = null;
  }
  if (canvasEl) {
    canvasEl.style.width = '';
    canvasEl.style.height = '';
  }
  state.playLogs = [];
  state.sheetOpen = false;
  state.info = null;
  session = null;
  state.page = 'load';
  // 回到加载页就清掉深链参数，避免刷新时又自动进入上一个预览。
  window.history.replaceState(null, '', window.location.pathname);
}

/** 音频开始出声：静音状态与手势监听都在这里收尾，保证角标和真实播放状态一致。 */
audio.addEventListener('playing', () => {
  markAudioAudible();
  removeGestureHints();
});

/** 标签页切回前台时尝试续播：后台标签的音频会被浏览器挂起。 */
document.addEventListener('visibilitychange', () => {
  if (document.visibilityState !== 'visible' || !state.playing || audioEnded) return;
  if (!audio.paused) return;
  startAudio();
});

/** 显示页在加载后自动播放：音频事件只用来推进「是不是已经放完」这一状态。 */
audio.addEventListener('ended', () => {
  // 音频可能比谱面短，结束后继续用 requestAnimationFrame 驱动画面时钟。
  audioEnded = true;
});

/**
 * 元数据到位后把之前记下的目标进度落实下去。
 *
 * 只补「已经记过目标但还没下发」的情况，避免在别的 seek 还在飞时插队。
 */
audio.addEventListener('loadedmetadata', () => {
  if (!session || audioSeekPending || !Number.isFinite(lastSeekedMs)) return;
  syncAudioToPosition();
});

audio.addEventListener('error', () => {
  if (!audio.src) return;
  state.status = '音频错误';
  logPlay(`音频失败：${audio.error?.message || '音频无法解码或播放'}`);
});

targetFps = state.fps;
