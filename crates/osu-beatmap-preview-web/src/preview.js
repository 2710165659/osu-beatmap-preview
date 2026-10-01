// 预览页的共享状态与全部播放逻辑。
//
// 组件只负责画界面：谱面会话（WASM）、音频输出、播放时钟、Mod 与分辨率切换都集中
// 在这里。state 做成模块级单例，组件直接引用，避免为了几个控件层层传递 props。
//
// 输入只有**一份文件**（`.osu` 或 `.osz` 整包字节）：`.osz` 的解包、音乐/背景/音效的
// 解码、统一混音与时钟全部在 WASM 里完成；宿主只做四件事——取字节、给 Canvas、把混音
// 结果写进音频环形缓冲、把音频线程的消费位置转发回 WASM。因此这里**没有 `<audio>`
// 元素、没有静音 WAV 假时钟**：画面时间直接来自 `session.clockMs()`，音画共用一条时间轴。

import { computed, nextTick, reactive } from 'vue';

import { createAudioContext, createAudioOutput } from './hitsound.js';

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

/** 数字模式 → 谱面模式 key（`hitsoundDefaults` 的参数）。 */
const MODE_KEYS = ['standard', 'taiko', 'catch', 'mania'];

/**
 * 默认音量 50%。
 *
 * 网页预览通常和其他标签页/应用的声音同时存在，默认拉满容易盖过别的声音，
 * 所以留半个刻度。用户调过之后不再重置：换谱面、切 Mod 都沿用当前音量。
 */
const DEFAULT_VOLUME = 0.5;

/**
 * 打击音（hit sound）默认为开，音量 50%。
 *
 * 50% 与网页的音乐默认音量 [`DEFAULT_VOLUME`] 取同一刻度；CLI 输出视频走
 * `shared_config.yml` 的 100%，那是另一套（音乐也满音量）的场景。
 *
 * 与音乐音量分开：用户可能想只听音乐、或只听打击音，两者的用途不同。
 */
const DEFAULT_HITSOUND_ENABLED = true;
const DEFAULT_HITSOUND_VOLUME = 50;

/**
 * 音频线程累计读到这么多帧静音（48kHz 下约 0.5 秒）才判定为「主线程没跟上混音」。
 *
 * 播放刚开始、seek 落地的那一两个音频块出现少量静音是正常的，阈值取大一些才能把
 * 真正的欠载和这些瞬态区分开。
 */
const AUDIO_UNDERRUN_ALERT_FRAMES = 24000;

/** 触发播放需要「用户激活」，因此只用指针 / 触摸 / 键盘这类真实输入当作手势。 */
const GESTURE_EVENTS = ['pointerdown', 'touchstart', 'keydown'];

/** 手势恢复音频后，多短时间内的播放/暂停操作算「只是想把声音打开」。 */
const GESTURE_RESUME_WINDOW = 400;

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

/**
 * 音频播放内核（AudioWorklet）。
 *
 * 放在 `public/` 下按运行时 URL 加载：worklet 必须在音频线程里独立加载。
 * 音乐与音效的混音结果由 WASM 产出，这里只做按硬件时钟消费。
 */
const AUDIO_WORKLET_URL = `${import.meta.env.BASE_URL}hitsound-worklet.js`;

// ---------------------------------------------------------------------------
// 状态
// ---------------------------------------------------------------------------

export const state = reactive({
  page: 'load',
  bid: '',
  /** 本地上传的文件名；非空表示按本地文件加载。 */
  localFileName: '',
  /** 本地文件类型：`osu` / `osz` / 空。 */
  localKind: '',
  /** `.osz` 里的难度清单（`[{ entry, label, beatmapId }]`），加载页选择用。 */
  localDifficulties: [],
  /** 选中的难度条目名（`.osz` 专用）。 */
  localDifficulty: '',
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
  /** 音乐音量（0–1）；0 即静音，与「静音播放中」角标无关。 */
  volume: DEFAULT_VOLUME,
  /** 是否启用打击音（hit sound）。 */
  hitsound: DEFAULT_HITSOUND_ENABLED,
  /**
   * 打击音音量百分比（0–100）。
   *
   * 取值与 `shared_config.yml` 的 `HITSOUND_VOLUME` 同义，但默认值不同：那份配置
   * 给 CLI 输出视频用（音乐与打击音都是 100%），网页的音乐默认只有 50%，
   * 打击音跟随网页这个刻度。用户调过之后不再重置。
   */
  hitsoundVolume: DEFAULT_HITSOUND_VOLUME,
  /** 音频输出状态文案：用于在侧栏说明当前是「已启用 / 已关闭 / 不可用」。 */
  hitsoundStatus: '',
  /**
   * 是否使用谱面自带的自定义打击音（`ENABLE_BEATMAP_HITSOUND`）。
   *
   * 与打击音开关一样来自 `assets/shared_config.yml`（由 WASM 转出）；默认启用。
   * 它在会话创建时生效（决定装载哪些样本）。
   */
  hitsoundBeatmap: true,
  /** 界面上勾选的 Mod token；DA 提交时会展开成 DAAR..CS..。 */
  mods: [],
  /**
   * 当前模式可选的 Mod token，来自 core 的支持矩阵（`supportedMods`）。
   *
   * 以前这里放着一份手抄的支持表，新增 HD/FL 后没跟上，网页就点不到。
   * 现在由会话建立时按实际模式取一次，前端不再自带列表。
   */
  modOptions: [],
  daAr: 9,
  daCs: 4,
  sheetOpen: false,
  /**
   * 声音是否没有真的出来（自动播放策略把 AudioContext 拦下了）。
   *
   * 这个值描述的是「此刻音频真的没在响」：上下文恢复后必须归位，否则提示会一直挂着。
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
   * osu 服务端定位谱面 → osz 服务端下载谱面包 → transfer 传输到客户端 →
   * ready 进播放页（`.osz` 的解包与解码在 WASM 内完成）。
   */
  progress: { phase: 'idle', received: 0, total: 0, message: '', speed: 0, eta: null },
  gpuAvailable: typeof navigator !== 'undefined' && Boolean(navigator.gpu),
  /** 安全上下文（https / localhost）。WebGPU 只在这里可用，用它区分两种失败原因。 */
  secureContext: typeof window === 'undefined' || window.isSecureContext !== false,
});

/** 加载阶段的文案；未知阶段一律显示「正在加载」。 */
const PROGRESS_LABELS = {
  osu: '服务端定位谱面',
  osz: '服务端下载谱面包',
  transfer: '传输到客户端',
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

/** 当前模式支持的 Mod；会话建立前为空，不显示任何按钮。 */
export const modTokens = computed(() => state.modOptions);
/** DA 参数只在勾选 DA 后展开：默认隐藏，避免占掉抽屉里一大块位置。 */
export const daVisible = computed(() => modTokens.value.includes('DA') && state.mods.includes('DA'));

// 非响应式的内部状态：这些值每帧都会变，放进 reactive 只会带来无意义的依赖追踪。
let session = null;
let canvasEl = null;
let viewportEl = null;
let viewportObserver = null;
let wasmReady = null;
/** 已加载的 wasm 模块（`beatmapInfo` / `hitsoundDefaults` 都是自由函数）。 */
let wasmModule = null;
let animation = 0;
let absoluteStart = 0;
let targetFps = 60;
let lastRenderTime = 0;
let hasRenderedFrame = false;
/**
 * 音频输出（AudioContext + Worklet + 环形缓冲）。
 *
 * 只有「AudioWorklet + SharedArrayBuffer」都可用时才会建立；不可用时退化成
 * 「只有画面」，并把这个原因写进状态文案与播放日志。
 */
let audioOutput = null;
/** 音频被自动播放策略拦下时的提示只写一次日志，避免 tick 里的重试刷屏。 */
let audioBlockLogged = false;
/**
 * 用户手势恢复音频成功的时刻（performance.now()），0 表示本次手势没有恢复过。
 *
 * 恢复发生在 pointerdown 捕获阶段，而紧随其后的 click 会走到「点击画面 = 播放 /
 * 暂停」上：如果用户只是想把声音打开，那一次点击不该顺带把画面也暂停掉，所以
 * 点击处理会先消费掉这个标记。
 */
let audioResumedAt = 0;
let modSwitching = false;
let appliedMods = [];
/**
 * 加载令牌：每次 `loadPreview()` 递增。
 *
 * 每个 await 之后都要用令牌确认自己还是「当前这次加载」，否则旧请求回来会把新会话的
 * 状态覆盖掉（用户可能又加载了别的谱面、或者退回加载页）。
 */
let loadToken = 0;
/** 当前加载的取消句柄：切谱面或退回加载页时中止还在飞的请求。 */
let loadAbort = null;
/**
 * 本地上传的文件（`File` 对象）与它的字节；`null` 表示按 BID 从后端加载。
 *
 * 不放进 reactive：`File` 参与响应式没有意义，界面只需要 state 里的展示字段。
 * 字节在选文件时读一次（出难度清单），加载时复用，避免同一大文件读两遍。
 */
let localFile = null;
let localBytes = null;
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
/** 加载进度的轮询计时器。 */
let progressTimer = 0;

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

/** 当前用户倍速（不含 DT/HT；总倍速由 WASM 乘上谱面变速）。 */
const userRate = () => speedOverride ?? state.speed;

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
 * 它。典型场景是前端已经进了播放页，而轮询仍在下发更早的 osz——同一份数据在两条
 * 链路上流动，谁最新以本地为准。
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
 * 云端部署时「慢」可能来自镜像、服务端下载或本地带宽，把这几个数字摊开才判断得出
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
// WASM 与文件字节
// ---------------------------------------------------------------------------

async function loadWasm() {
  if (!wasmReady) {
    // 失败时清空缓存，下一次点击「加载预览」还能重新尝试。
    wasmReady = import(/* @vite-ignore */ WASM_URL)
      .then(async (module) => {
        await module.default();
        wasmModule = module;
        return module;
      })
      .catch((error) => {
        wasmReady = null;
        wasmModule = null;
        throw error;
      });
  }
  return wasmReady;
}

/** 从后端下载整份谱面包（`.osz` 字节，带字节进度）。 */
async function fetchFile(value, { signal } = {}) {
  return fetchWithProgress(`/resource/file?bid=${encodeURIComponent(value)}`, {
    signal,
    onProgress: ({ received, total }) => reportProgress({
      phase: 'transfer', received, total, message: '传输到客户端', source: 'local',
    }),
  });
}

/**
 * 读取谱面内部信息与难度清单。
 *
 * WASM 只按传入的文件字节解析（`.osu` 或 `.osz` 都行），`selector` 决定 `.osz` 里
 * 选哪个难度。失败只记日志，不影响预览本身。
 */
function readBeatmapInfo(wasm, bytes, selector) {
  try {
    return wasm.beatmapInfo(bytes, selector);
  } catch (error) {
    logPlay(`谱面信息解析失败：${errorText(error)}`);
    return null;
  }
}

// ---------------------------------------------------------------------------
// 本地文件（.osu / .osz）
// ---------------------------------------------------------------------------

/** 本地输入文件类型（按扩展名，不区分大小写）；WASM 再按内容复核。 */
function localFileKind(name) {
  const text = String(name ?? '');
  const dot = text.lastIndexOf('.');
  const extension = dot >= 0 ? text.slice(dot + 1).toLowerCase() : '';
  if (extension === 'osu') return 'osu';
  if (extension === 'osz') return 'osz';
  return null;
}

/**
 * 选择本地文件，回到加载页时调用（文件输入框的 change 处理）。
 *
 * `.osz` 会顺手把难度清单解析出来供界面选择（清单来自 WASM 的 `beatmapInfo`，
 * 解包规则与 CLI 同一套）；填了 BID 时默认选中对应 `BeatmapID` 的难度。
 * 文件不合法直接抛错，由加载页显示。
 *
 * @param {File} file 用户选中的文件
 */
export async function selectLocalFile(file) {
  const kind = localFileKind(file?.name ?? '');
  if (!kind) throw new Error('只支持 .osu 或 .osz 文件');
  localFile = file;
  localBytes = new Uint8Array(await file.arrayBuffer());
  state.localFileName = file.name;
  state.localKind = kind;
  state.localDifficulties = [];
  state.localDifficulty = '';
  if (kind !== 'osz') return;
  const wasm = await loadWasm();
  const info = readBeatmapInfo(wasm, localBytes, {});
  const difficulties = Array.isArray(info?.difficulties) ? info.difficulties : [];
  if (!difficulties.length) throw new Error(`${file.name} 里找不到 .osu 谱面文件`);
  state.localDifficulties = difficulties;
  const bid = state.bid.trim();
  const matched = bid ? difficulties.find((item) => String(item.beatmapId) === bid) : null;
  state.localDifficulty = (matched ?? difficulties[0]).entry;
}

/** 清除本地文件选择，回到按 BID 加载。 */
export function clearLocalFile() {
  localFile = null;
  localBytes = null;
  state.localFileName = '';
  state.localKind = '';
  state.localDifficulties = [];
  state.localDifficulty = '';
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

// ---------------------------------------------------------------------------
// 会话与音频输出
// ---------------------------------------------------------------------------

/** 会话重建（加载、切 Mod）后同步时长与时间轴换算基准。 */
function applySessionMetrics() {
  state.duration = Math.max(1, session.durationMs());
  absoluteStart = session.absoluteStartMs();
}

/** 数字模式 → `hitsoundDefaults` 的模式 key。 */
function modeKeyOf(mode) {
  return MODE_KEYS[Number(mode) | 0] ?? 'standard';
}

/**
 * 读取共享配置里该模式「是否启用打击音」与「是否使用谱面自带音效」的默认值。
 *
 * 走 WASM 转出的 `hitsoundDefaults`（内部来自 `assets/shared_config.yml`），
 * 读不到就沿用前端常量，保证页面仍能正常工作。**必须在会话创建之前调用**：
 * 「是否使用谱面自带音效」决定 WASM 装载哪些样本，创建之后改不了。
 *
 * 音量**不**取配置里的值：CLI 输出视频时背景音乐与打击音都是满音量（100%），
 * 而网页的音乐默认是 [`DEFAULT_VOLUME`]（50%），打击音跟着取同一个刻度听感才平衡，
 * 因此这里保持 [`DEFAULT_HITSOUND_VOLUME`]，只同步开关。
 */
function applyHitsoundDefaults(wasm, mode) {
  let defaults = null;
  try {
    defaults = wasm.hitsoundDefaults?.(mode) ?? null;
  } catch (error) {
    logPlay(`打击音默认配置读取失败：${errorText(error)}`);
  }
  if (!defaults) return;
  state.hitsound = Boolean(defaults.enabled);
  // 旧版 wasm 没有这个字段：缺省按「启用谱面自带音效」处理（与配置默认值一致）。
  state.hitsoundBeatmap = defaults.beatmapEnabled !== false;
}

/** 释放音频输出与它的 AudioContext，音频线程不残留。 */
function releaseAudioOutput() {
  if (!audioOutput) return;
  audioOutput.setPlaying(false);
  audioOutput.close();
  audioOutput.context?.close().catch(() => {});
  audioOutput = null;
}

/**
 * 取当前模式可选的 Mod 列表。列表由 core 的支持矩阵转出，前端不再自带一份：
 * 以前手抄的支持表在新增 HD/FL 之后没跟上，网页就点不到这两个 Mod。
 *
 * 会话创建之后调用（`mode` 取 `session.mode()`，即转谱后的真实模式）。
 * `supportedMods` 缺失表示 wasm 产物比页面旧，此时列表为空并留下可操作的日志。
 */
function applyModOptions(wasm, mode) {
  try {
    state.modOptions = Array.from(wasm.supportedMods?.(mode) ?? []);
  } catch (error) {
    logPlay(`Mod 列表读取失败：${errorText(error)}`);
    state.modOptions = [];
  }
  if (state.modOptions.length === 0) {
    logPlay(`当前 wasm 产物没有 ${mode} 的 Mod 列表（supportedMods），请重新构建 pkg`);
  }
}

/** 会话整体释放（换谱面、退回加载页）。 */
function teardownSession() {
  releaseAudioOutput();
  session = null;
  state.rendered = false;
  state.renderError = '';
  // 列表随会话一起失效：下一次加载会按新模式的矩阵重新取。
  state.modOptions = [];
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

/** 按 WASM 内部时钟渲染一帧（时钟由会话维护，宿主不再算时间）。 */
function render() {
  if (!session) return;
  try {
    session.renderFrame();
    state.rendered = true;
    state.renderError = '';
  } catch (error) {
    playState(false);
    state.renderError = '渲染失败，请查看日志';
    logPlay(`渲染失败：${errorText(error)}`);
  }
}

/** 把画面进度从会话时钟同步过来（时钟是唯一时间权威）。 */
function syncPositionFromClock() {
  const position = session.clockMs() - absoluteStart;
  state.position = Math.min(state.duration, Math.max(0, position));
}

/**
 * 让音频输出跟上播放状态。
 *
 * 声音只在「画面在播 + AudioContext 在跑」时输出：上下文被自动播放策略挂起时
 * 环形缓冲照常预混（时钟继续走），恢复后从画面位置接上，不会把暂停期间的声音补播出来。
 */
function syncAudioOutput() {
  if (!audioOutput) return;
  const context = audioOutput.context;
  const running = context.state === 'running';
  const shouldPlay = state.playing && running;
  audioOutput.setPlaying(shouldPlay);
  // 「声音没出来」只看上下文是否在跑：恢复后角标必须自己归位。
  state.audioBlocked = state.playing && !running;
  if (state.playing && !running) {
    // 自动播放策略：恢复只能发生在用户手势里（handleUserGesture），这里只是幂等重试。
    context.resume().catch(() => {});
    if (!audioBlockLogged) {
      audioBlockLogged = true;
      logPlay('浏览器拦截了自动播放，点一下画面或按任意键即可播放声音。');
    }
  }
  if (running) audioBlockLogged = false;
}

/**
 * 补足音频缓冲并做欠载诊断：每帧调用一次。
 *
 * WASM 内部决定「混到哪、混多少」并把结果直接给出，宿主只负责写进环形缓冲。
 */
function updateAudio() {
  if (!audioOutput || !state.playing) return;
  audioOutput.ensureBuffered();
  // 音频线程读到静音说明主线程没跟上混音，声音会断续。只提示一次：这条日志是排查
  // 「音效时有时无」最直接的线索，但持续刷屏没有意义。
  if (audioOutput.underrunFrames >= AUDIO_UNDERRUN_ALERT_FRAMES) {
    audioOutput.underrunFrames = 0;
    logPlay('音频缓冲跟不上播放，声音可能断续。');
  }
}

/** 播到结尾：停在最后一帧，等用户再点播放。 */
function finishPlayback() {
  state.position = state.duration;
  if (state.playing) playState(false);
}

/** 从结尾重新开始时先把时钟拨回开头，否则第一帧又会判定结束。 */
function restartPlayback() {
  session.seek(absoluteStart);
  syncPositionFromClock();
  render();
}

function playState(next) {
  state.playing = next;
  lastRenderTime = 0;
  hasRenderedFrame = false;
  cancelAnimationFrame(animation);
  if (!session) return;
  if (next) {
    session.play();
    syncAudioOutput();
    updateAudio();
    animation = requestAnimationFrame(tick);
  } else {
    // 先把进度钉在时钟当下所在的位置再暂停，恢复时才不会跳回上一帧。
    session.pause();
    syncPositionFromClock();
    syncAudioOutput();
    render();
  }
}

function tick(now) {
  if (!state.playing) return;
  if (!session) return;

  // 时钟是唯一时间权威：画面、音频、进度条都从这里取值。
  syncPositionFromClock();
  syncAudioOutput();
  updateAudio();

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
 * 输入只有一份文件：本地 `.osu` / `.osz` 直接读字节，BID 由后端下载整包。文件进了
 * WASM 之后，解包、解码、会话建立都在里面完成，因此这里没有「先出画面再补音频」的
 * 并行阶段。每个 await 之后都用 loadToken 确认自己还是当前这次加载。
 */
export async function loadPreview() {
  if (state.loading) return;
  const bid = state.bid.trim();
  if (!localFile && !/^\d+$/.test(bid)) {
    logLoad('请输入数字 BID，或选择本地 .osu / .osz 文件');
    return;
  }
  state.loadLogs = [];
  state.loading = true;
  loadAbort?.abort();
  loadAbort = new AbortController();
  const token = ++loadToken;
  resetProgress(localFile ? 'transfer' : 'osu');
  teardownSession();
  state.hitsoundStatus = '';
  try {
    const wasm = await loadWasm();
    if (token !== loadToken) return;

    // 1. 取一份文件的字节。
    let bytes;
    if (localFile) {
      bytes = localBytes ?? new Uint8Array(await localFile.arrayBuffer());
      reportProgress({
        phase: 'transfer',
        received: bytes.byteLength,
        total: bytes.byteLength,
        message: '读取本地文件',
        source: 'local',
      });
    } else {
      startProgressPolling(bid);
      bytes = await fetchFile(bid, { signal: loadAbort.signal });
    }
    if (token !== loadToken) return;

    // 2. 难度选择：本地 `.osz` 用下拉选中的条目，BID 按 `BeatmapID` 匹配。
    const selector = localFile
      ? (state.localDifficulty ? { difficulty: state.localDifficulty } : {})
      : { bid };
    state.info = readBeatmapInfo(wasm, bytes, selector);
    // 3. 打击音默认值要在创建会话前就位：它决定 WASM 装载哪些样本。
    applyHitsoundDefaults(wasm, modeKeyOf(state.info?.mode));

    // 4. 音频输出先建好：设备采样率要交给 WASM，混音结果才能直接播放。
    const audioContext = await createAudioContext(AUDIO_WORKLET_URL);
    if (token !== loadToken) {
      await audioContext?.close().catch(() => {});
      return;
    }

    // 5. 文件进 WASM：解包、解码、混音、时钟都在里面。
    state.status = '准备会话...';
    const { width, height } = RESOLUTIONS[state.resolution];
    session = await wasm.WebGpuSession.create(bytes, canvasEl, {
      ...selector,
      convert: state.convert || undefined,
      mods: [],
      width,
      height,
      sampleRate: audioContext?.sampleRate ?? 48000,
      hitsoundEnabled: state.hitsound,
      hitsoundVolume: state.hitsoundVolume,
      musicVolume: Math.round(state.volume * 100),
      beatmapHitsound: state.hitsoundBeatmap,
    });
    if (token !== loadToken) {
      await audioContext?.close().catch(() => {});
      return;
    }
    appliedMods = [];
    state.mods = [];
    state.renderError = '';
    state.rendered = false;
    state.audioBlocked = false;
    audioBlockLogged = false;
    applySessionMetrics();
    session.setRate(userRate());
    state.position = 0;
    state.modeKey = session.mode();
    state.mode = state.modeKey.toUpperCase();
    // Mod 面板按实际模式（含转谱结果）取一次；HD/FL 是否可选由 core 决定。
    applyModOptions(wasm, state.modeKey);
    state.status = hasMusic() ? '就绪' : '无音乐（.osu 单文件）';

    // 6. 接上音频输出；不可用就退化成只有画面。
    audioOutput = audioContext ? createAudioOutput({ session, context: audioContext }) : null;
    if (audioContext && !audioOutput) {
      await audioContext.close().catch(() => {});
      state.hitsoundStatus = '音频输出不可用（AudioWorklet 节点创建失败）';
      logPlay('音频输出不可用，将继续只播放画面');
    } else if (!audioContext) {
      state.hitsoundStatus = '当前浏览器不支持音频输出（需要 AudioWorklet 与 SharedArrayBuffer）';
      logPlay('音频输出不可用：AudioWorklet 或 SharedArrayBuffer 缺失，将继续只播放画面');
    } else {
      audioOutput.setRate(session.rate());
      state.hitsoundStatus = state.hitsound ? '已启用' : '已关闭';
    }

    canvasEl.width = session.width();
    canvasEl.height = session.height();
    state.page = 'play';
    state.sheetOpen = false;
    // 等播放页真正显示出来再量尺寸，否则 clientWidth/Height 还是 0。
    await nextTick();
    fitCanvasToViewport();
    showTimeline();
    render();
    reportProgress({ phase: 'ready', source: 'local' });
    logStageBreakdown();
    // 加载完成后直接进入播放状态：已经在页面上点过「加载预览」时这次激活允许出声，
    // 浏览器若仍按自动播放策略拦下 AudioContext，画面时钟照常推进，并由手势监听
    // 等待用户激活（首次点击 / 触摸 / 按键）后恢复声音。
    playState(true);
    addGestureHints();
    // 本地文件没有可分享的深链，地址栏保持不变。
    if (!localFile) syncUrl();
  } catch (error) {
    // 取消或已经被新的一次加载取代：错误属于旧请求，不该报给用户。
    if (token !== loadToken || error?.name === 'AbortError') return;
    logLoad(error);
  } finally {
    // 被新加载取代或退回了加载页时不能碰 state：那两个入口自己会收拾。
    if (token === loadToken) {
      stopProgressPolling();
      state.loading = false;
    }
  }
}

/** 当前来源有没有音乐：`.osz` 与 BID 的整包里都有，单独的 `.osu` 没有。 */
function hasMusic() {
  return !(localFile && state.localKind === 'osu');
}

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

/**
 * 合并服务端上报的进度。
 *
 * 服务端只知道「自己下到哪了」，浏览器接收字节的进度由前端自己统计（transfer 阶段），
 * 因此这里不能反过来覆盖掉本地阶段：谁的信息更靠后就以谁为准，`PHASE_ORDER`
 * 就是这个先后关系。
 */
function applyProgress(payload) {
  const phase = typeof payload?.phase === 'string' ? payload.phase : 'idle';
  if (phase === 'idle' || phase === 'ready' || phase === 'error') {
    // ready 由前端在真正进入播放页时自己上报，避免服务端提前把进度条推到终点。
    if (phase === 'error') reportProgress({ phase: 'error', message: String(payload?.message ?? '') });
    return;
  }
  // 前端已经自己推进过阶段（例如开始接收字节）时，服务端轮询回来的旧阶段一律忽略。
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
const PHASE_ORDER = ['idle', 'osu', 'osz', 'transfer', 'ready', 'error'];
const phaseOrder = (phase) => {
  const index = PHASE_ORDER.indexOf(phase);
  return index < 0 ? 0 : index;
};

/**
 * 判断这次播放 / 暂停操作是否是「刚刚被用来开启声音的那一次」，是则消费掉。
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
  // 播到结尾后再点播放要从头开始：否则第一帧就会再次判定结束并立刻暂停。
  if (state.position >= state.duration - 1) restartPlayback();
  playState(true);
}

export function seekTo(nextPosition) {
  if (!session) return;
  const resume = state.playing;
  if (resume) playState(false);
  const target = Math.min(state.duration, Math.max(0, nextPosition));
  session.seek(absoluteStart + target);
  syncPositionFromClock();
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
  applyRate();
}

function startFastForward() {
  state.fastForwarding = true;
  speedOverride = FAST_FORWARD_SPEED;
  applyRate();
}

function stopFastForward() {
  if (!state.fastForwarding) return;
  state.fastForwarding = false;
  speedOverride = null;
  applyRate();
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
 * 而墙钟不会停下来——不冻结的话操作结束后画面已经跑到前面。时钟在 WASM 里，
 * 所以这里直接「暂停 → 改参数 → 恢复」，没有任何额外对齐。
 */
function withFrozenClock(mutate) {
  const resume = state.playing;
  if (resume) playState(false);
  try {
    mutate();
  } finally {
    if (resume) playState(true);
  }
}

/** 倍速变化：会话换速（内部乘上谱面变速），音频线程按新的总倍速消费。 */
function applyRate() {
  if (!session) return;
  session.setRate(userRate());
  audioOutput?.setRate(session.rate());
}

export function setSpeed(value) {
  state.speed = value;
  applyRate();
}

/**
 * 调整音乐音量。
 *
 * 音量即时生效，写 WASM 的音乐增益即可；0–1 先夹紧再换算成百分比。
 */
export function setVolume(value) {
  const number = Number(value);
  if (!Number.isFinite(number)) return;
  state.volume = Math.min(1, Math.max(0, number));
  try {
    session?.setMusicVolume(Math.round(state.volume * 100));
  } catch (error) {
    logPlay(`音乐音量设置失败：${errorText(error)}`);
  }
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
      render();
    } catch (error) {
      state.resolution = previous;
      logPlay(`分辨率切换失败：${errorText(error)}`);
    }
  });
}

/** 切换是否启用打击音；关闭时音乐照常输出。 */
export function setHitsoundEnabled(value) {
  state.hitsound = Boolean(value);
  if (!session) return;
  try {
    session.setHitsoundEnabled(state.hitsound);
  } catch (error) {
    state.hitsound = false;
    state.hitsoundStatus = `打击音启用失败：${errorText(error)}`;
    return;
  }
  state.hitsoundStatus = state.hitsound ? '已启用' : '已关闭';
}

/**
 * 更新打击音音量（0–100）。
 *
 * 只写 WASM 的混音增益，不重建会话、不重新加载样本，滑动过程中即时生效。
 */
export function setHitsoundVolume(value) {
  const number = Number(value);
  if (!Number.isFinite(number)) return;
  state.hitsoundVolume = Math.min(100, Math.max(0, Math.round(number)));
  try {
    session?.setHitsoundVolume(state.hitsoundVolume);
  } catch (error) {
    logPlay(`打击音音量设置失败：${errorText(error)}`);
  }
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
    // 转谱会改变时间轴、总倍速与所需样本，样本由 WASM 内部按新谱面重新装载。
    session.set_mods(submitted);
    appliedMods = requested;
    applySessionMetrics();
    state.position = Math.min(state.position, state.duration);
    applyRate();
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
  teardownSession();
  // 还在飞的下载要一并中止：否则它回来时会往已经清空的播放页写状态。
  loadAbort?.abort();
  loadAbort = null;
  loadToken += 1;
  stopProgressPolling();
  resetProgress();
  state.loading = false;
  audioResumedAt = 0;
  state.audioBlocked = false;
  audioBlockLogged = false;
  state.hitsoundStatus = '';
  if (canvasEl) {
    canvasEl.style.width = '';
    canvasEl.style.height = '';
  }
  state.playLogs = [];
  state.sheetOpen = false;
  state.info = null;
  state.page = 'load';
  // 回到加载页就清掉深链参数，避免刷新时又自动进入上一个预览。
  window.history.replaceState(null, '', window.location.pathname);
}

/**
 * 在用户手势里恢复音频。
 *
 * 自动播放被拦下后，只有用户激活（指针 / 触摸 / 键盘）里的 `resume()` 才会被放行。
 * 恢复成功时记下时刻，让紧随其后的 click 不要顺手把画面暂停掉。
 */
function resumeAudioFromGesture() {
  const context = audioOutput?.context;
  if (!context || !state.playing) return Promise.resolve(false);
  if (context.state === 'running') return Promise.resolve(true);
  return context.resume()
    .then(() => {
      const running = context.state === 'running';
      if (running) {
        state.audioBlocked = false;
        audioBlockLogged = false;
        syncAudioOutput();
        audioResumedAt = performance.now();
      }
      return running;
    })
    .catch(() => false);
}

/**
 * 首次真实输入（指针 / 触摸 / 键盘）时恢复音频。
 *
 * 用捕获阶段监听：手势必须在事件处理的最前面把恢复请求发出去，才能算「由用户
 * 激活触发的播放」，后面的 click 也来不及把这次激活用掉。
 */
function handleUserGesture() {
  if (!state.playing) return;
  resumeAudioFromGesture().then((resumed) => {
    // 恢复成功就解除监听：音频回到暂停只可能是用户自己按的暂停，
    // 那时再自动恢复会对着干。
    if (resumed) removeGestureHints();
  });
}

function addGestureHints() {
  for (const type of GESTURE_EVENTS) window.addEventListener(type, handleUserGesture, { capture: true, passive: true });
}

function removeGestureHints() {
  for (const type of GESTURE_EVENTS) window.removeEventListener(type, handleUserGesture, { capture: true });
}

/** 标签页切回前台时尝试续播：后台标签的音频上下文会被浏览器挂起。 */
document.addEventListener('visibilitychange', () => {
  if (document.visibilityState !== 'visible' || !state.playing) return;
  resumeAudioFromGesture();
});

targetFps = state.fps;
