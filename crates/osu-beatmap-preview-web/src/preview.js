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
  audioFailed: false,
  /** 进度条是否可见：隐藏时只改透明度，盒子留在原处，画面不会上下跳。 */
  timelineVisible: true,
  /** 长按方向键的状态，用于在画面上给出反馈。 */
  fastForwarding: false,
  rewinding: false,
  /** WASM 解析出的谱面内部信息（`beatmapInfo` 全量输出）；未加载时为 null。 */
  info: null,
  /** 加载进度：percent 为 null 表示还不知道总量，用不确定态进度条。 */
  progress: { phase: 'idle', percent: null, detail: '' },
  gpuAvailable: typeof navigator !== 'undefined' && Boolean(navigator.gpu),
});

/** 加载阶段的文案；未知阶段一律显示「正在加载」。 */
const PROGRESS_LABELS = {
  osu: '获取谱面',
  osz: '下载谱面包',
  extract: '解析资源',
  ready: '准备渲染',
};

export const progressLabel = computed(() => PROGRESS_LABELS[state.progress.phase] ?? '正在加载');

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
let pendingAudioTime = null;
let audioEnded = false;
let ignoreAudioUntil = 0;
let audioFailureLogged = false;
let lastAudioStartAttempt = 0;
let audioObjectUrl = null;
let modSwitching = false;
let appliedMods = [];
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

async function fetchBeatmap(value) {
  const response = await fetch(`/resource/beatmap?bid=${encodeURIComponent(value)}`);
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

const formatBytes = (bytes) => {
  const mib = bytes / 1024 / 1024;
  if (mib >= 1) return `${mib.toFixed(1)} MiB`;
  return `${Math.max(1, Math.round(bytes / 1024))} KiB`;
};

function applyProgress(payload) {
  const phase = typeof payload?.phase === 'string' ? payload.phase : 'idle';
  const received = Math.max(0, Number(payload?.received) || 0);
  const total = Math.max(0, Number(payload?.total) || 0);
  state.progress.phase = phase;
  if (phase === 'osz' && total > 0) {
    // 下载阶段给到 99% 就够：剩下的解压与建会话不该让进度条一直停在 100%。
    state.progress.percent = Math.min(99, Math.round((received / total) * 100));
    state.progress.detail = `${formatBytes(received)} / ${formatBytes(total)}`;
    return;
  }
  state.progress.percent = null;
  state.progress.detail = phase === 'osz' && received > 0 ? formatBytes(received) : '';
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

async function loadBackground(value) {
  const response = await fetch(`/resource/background?bid=${encodeURIComponent(value)}`);
  if (!response.ok) throw new Error(await response.text());
  const bitmap = await createImageBitmap(await response.blob());
  if (backgroundBitmap) backgroundBitmap.close();
  backgroundBitmap = bitmap;
  paintBackground();
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

async function loadAudioBlob(value) {
  const response = await fetch(`/resource/audio?bid=${encodeURIComponent(value)}`);
  if (!response.ok) throw new Error(await response.text());
  const blob = await response.blob();
  if (!blob.size) throw new Error('音频资源为空');
  releaseAudioUrl();
  audioObjectUrl = URL.createObjectURL(blob);
  audio.src = audioObjectUrl;
  audio.load();
  await new Promise((resolve, reject) => {
    const ready = () => { cleanup(); resolve(); };
    const failed = () => { cleanup(); reject(new Error(audio.error?.message || '音频无法解码或播放')); };
    const cleanup = () => {
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
 * HTMLMediaElement 的 currentTime 是异步 seek，短时间内不能用旧值覆盖新进度，
 * 因此设置 ignoreAudioUntil 让时钟短暂地走 rAF 而不是音频。
 */
function syncAudioToPosition() {
  const time = absoluteTime();
  audioEnded = false;
  ignoreAudioUntil = performance.now() + 250;
  if (time < 0) {
    audio.pause();
    pendingAudioTime = 0;
    if (audio.readyState >= HTMLMediaElement.HAVE_METADATA) audio.currentTime = 0;
    return;
  }
  pendingAudioTime = time / 1000;
  if (Number.isFinite(audio.duration)) {
    audio.currentTime = Math.min(pendingAudioTime, audio.duration);
    pendingAudioTime = null;
  }
}

/**
 * 请求播放音频。
 *
 * 播到结尾、暂停或 seek 都会让上一次尚未落地的 play() 被 pause() 打断，
 * 此时浏览器会抛 AbortError。这属于正常流程，只有真正播不出来才写日志。
 */
function startAudio() {
  if (!audio.src) return;
  if (absoluteTime() < 0 || audioEnded) return;
  lastAudioStartAttempt = performance.now();
  const request = audio.play();
  if (!request) return;
  request.catch((error) => {
    if (!state.playing || error?.name === 'AbortError') return;
    if (audioFailureLogged) return;
    audioFailureLogged = true;
    state.audioFailed = true;
    if (error?.name === 'NotAllowedError') {
      logPlay('浏览器拦截了自动播放，点一下画面即可播放声音。');
      return;
    }
    logPlay(`音频播放失败，继续播放画面：${errorText(error)}`);
  });
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
    startAudio();
    animation = requestAnimationFrame(tick);
  } else {
    audio.pause();
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
  ignoreAudioUntil = 0;
  pendingAudioTime = 0;
  syncAudioToPosition();
  render();
}

function tick(now) {
  if (!state.playing) return;
  if (!lastTick) lastTick = now;
  const delta = now - lastTick;
  lastTick = now;

  if (absoluteTime() < 0 || audio.paused || audioEnded || now < ignoreAudioUntil) {
    state.position = Math.min(state.duration, state.position + delta * playbackRate());
    // 音频被自动播放策略拦下或还在缓冲时，画面继续走，并定期重试播放。
    if (absoluteTime() >= 0 && audio.paused && !audioEnded
      && now - lastAudioStartAttempt >= AUDIO_RETRY_INTERVAL) {
      syncAudioToPosition();
      startAudio();
    }
  } else {
    const audioPosition = audio.currentTime * 1000 - absoluteStart;
    // seek 或浏览器缓冲期间 currentTime 可能暂时回到旧值；只接受接近当前进度的时钟。
    if (Number.isFinite(audioPosition) && Math.abs(audioPosition - state.position) < 1500) {
      state.position = Math.min(state.duration, Math.max(0, audioPosition));
    } else {
      state.position = Math.min(state.duration, state.position + delta * playbackRate());
    }
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

export async function loadPreview() {
  if (state.loading) return;
  state.loadLogs = [];
  state.loading = true;
  state.info = null;
  cancelArrowHold();
  state.progress = { phase: 'osu', percent: null, detail: '' };
  try {
    const bid = state.bid.trim();
    if (!bid) throw new Error('请输入谱面 BID');
    if (!canvasEl) throw new Error('画布尚未就绪，请刷新页面重试');
    state.bid = bid;
    startProgressPolling(bid);
    const [wasm, bytes] = await Promise.all([loadWasm(), fetchBeatmap(bid)]);
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
    appliedMods = [];
    state.mods = [];
    state.renderError = '';
    state.rendered = false;
    state.audioFailed = false;
    audioFailureLogged = false;
    audioEnded = false;
    lastAudioStartAttempt = 0;
    applySessionMetrics();
    state.position = 0;
    state.modeKey = session.mode();
    state.mode = state.modeKey.toUpperCase();
    state.status = '加载背景和音频...';
    const [backgroundResult, audioResult] = await Promise.allSettled([
      loadBackground(bid),
      loadAudioBlob(bid),
    ]);
    if (backgroundResult.status === 'rejected') {
      logPlay(`背景加载失败，将使用黑色背景：${errorText(backgroundResult.reason)}`);
    }
    if (audioResult.status === 'rejected') {
      logPlay(`音频加载失败，将只播放画面：${errorText(audioResult.reason)}`);
    }
    canvasEl.width = session.width();
    canvasEl.height = session.height();
    syncAudioToPosition();
    state.page = 'play';
    state.sheetOpen = false;
    state.status = audioResult.status === 'fulfilled' ? '就绪' : '无音频';
    // 等播放页真正显示出来再量尺寸，否则 clientWidth/Height 还是 0。
    await nextTick();
    fitCanvasToViewport();
    showTimeline();
    render();
    // 加载完成后直接进入播放状态。浏览器可能因自动播放策略拒绝音频，
    // 但画面时钟仍由 requestAnimationFrame 继续推进。
    playState(true);
    syncUrl();
  } catch (error) {
    logLoad(error);
  } finally {
    stopProgressPolling();
    state.loading = false;
  }
}

export function togglePlay() {
  if (!session) return;
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
  syncAudioToPosition();
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

export function setSpeed(value) {
  state.speed = value;
  audio.playbackRate = playbackRate();
}

export function setFps(value) {
  state.fps = value;
  targetFps = value;
  lastRenderTime = 0;
  hasRenderedFrame = false;
  if (state.page === 'play') render();
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
  const resume = state.playing;
  if (resume) playState(false);
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
  } finally {
    if (resume) playState(true);
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

/** 显示页在加载后自动播放：音频事件只用来推进「是不是已经放完」这一状态。 */
audio.addEventListener('ended', () => {
  // 音频可能比谱面短，结束后继续用 requestAnimationFrame 驱动画面时钟。
  audioEnded = true;
});

audio.addEventListener('loadedmetadata', () => {
  if (pendingAudioTime !== null && Number.isFinite(audio.duration)) {
    audio.currentTime = Math.min(pendingAudioTime, audio.duration);
    pendingAudioTime = null;
  }
});

audio.addEventListener('error', () => {
  if (!audio.src) return;
  state.status = '音频错误';
  logPlay(`音频失败：${audio.error?.message || '音频无法解码或播放'}`);
});

targetFps = state.fps;
