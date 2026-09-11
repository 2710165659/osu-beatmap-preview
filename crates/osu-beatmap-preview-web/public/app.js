// 播放时只驱动本地 WASM 和浏览器音频，绝不轮询远程渲染帧。
const $ = selector => document.querySelector(selector);
const loadPage = $('#load-page');
const playPage = $('#play-page');
const form = $('#load-form');
const canvas = $('#canvas');
const audio = $('#audio');
const seek = $('#seek');
const play = $('#play');
const speed = $('#speed');
const fpsSelect = $('#fps');
const resolutionSelect = $('#resolution');
const current = $('#current');
const total = $('#total');
const empty = $('#empty');
const modControls = $('#mod-controls');
const daControls = $('#da-controls');
const daAr = $('#da-ar');
const daCs = $('#da-cs');
const daArValue = $('#da-ar-value');
const daCsValue = $('#da-cs-value');
const viewport = $('#viewport');
const playerControls = $('#player-controls');

// 画布分辨率只影响输出尺寸和合成阶段，谱面、mod 解析仍在 WASM/core 中完成。
const RESOLUTIONS = Object.freeze({
  '1080': { width: 1920, height: 1080 },
  '720': { width: 1280, height: 720 },
  '480': { width: 854, height: 480 },
});
const selectedResolution = () => RESOLUTIONS[resolutionSelect.value] || RESOLUTIONS['720'];
const selectedFps = () => Number(fpsSelect.value) || 60;
const resolutionKeyFor = (width, height) =>
  Object.keys(RESOLUTIONS).find(key => {
    const size = RESOLUTIONS[key];
    return size.width === width && size.height === height;
  }) || '720';

let session = null;
let backgroundBitmap = null;
let bid = '';
let playing = false;
let position = 0;
let duration = 1;
let absoluteStart = 0;
let beatmapSpeed = 1;
let lastTick = 0;
let lastRenderTime = 0;
let hasRenderedFrame = false;
let animation = 0;
let wasmReady = null;
let pendingAudioTime = null;
let audioEnded = false;
let ignoreAudioUntil = 0;
let audioFailureLogged = false;
let activeMods = [];
let modSwitching = false;
let controlsHideTimer = 0;
let viewportObserver = null;
let targetFps = selectedFps();
// 播放页沿用 osu! 视频预览的默认背景暗化：暗化 70%，保留 30% 亮度，
// 只处理背景资源，不改变 playfield 和其他平台的渲染配置。
const BACKGROUND_DIM = 0.7;

const formatTime = milliseconds => {
  const seconds = Math.max(0, Math.floor(milliseconds / 1000));
  return `${String(Math.floor(seconds / 60)).padStart(2, '0')}:${String(seconds % 60).padStart(2, '0')}`;
};
const errorText = error => {
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === 'string') return error;
  try { return JSON.stringify(error); } catch (_) { return String(error); }
};
const log = (target, message) => {
  target.textContent += `${new Date().toLocaleTimeString()} ${errorText(message)}\n`;
  target.scrollTop = target.scrollHeight;
};
const playbackRate = () => Number(speed.value) * beatmapSpeed;
const absoluteTime = () => absoluteStart + position;

// 控制条 HUD 自动隐藏逻辑：播放中无操作后淡出，暂停或指针在控制条上时保持。
function scheduleControlsHide(delay = 2600) {
  clearTimeout(controlsHideTimer);
  if (!playing) return;
  controlsHideTimer = setTimeout(() => {
    if (playerControls.matches(':hover') || playerControls.matches(':focus-within')) {
      scheduleControlsHide(1200);
      return;
    }
    viewport.classList.remove('controls-visible');
  }, delay);
}
function revealControls() {
  viewport.classList.add('controls-visible');
  scheduleControlsHide();
}
function keepControlsVisible() {
  clearTimeout(controlsHideTimer);
  viewport.classList.add('controls-visible');
}

// canvas 的 width/height 属性是 WASM/GPU 的像素尺寸；CSS 显示尺寸由这里按
// viewport 内容区做 contain 计算，避免浏览器只按宽度缩放导致底部被裁掉。
function fitCanvasToViewport() {
  if (!session || playPage.hidden) return;
  const style = getComputedStyle(viewport);
  const availableWidth = viewport.clientWidth
    - parseFloat(style.paddingLeft)
    - parseFloat(style.paddingRight);
  const availableHeight = viewport.clientHeight
    - parseFloat(style.paddingTop)
    - parseFloat(style.paddingBottom);
  if (availableWidth <= 0 || availableHeight <= 0) return;
  const sourceWidth = session.width();
  const sourceHeight = session.height();
  if (!(sourceWidth > 0 && sourceHeight > 0)) return;
  const scale = Math.min(availableWidth / sourceWidth, availableHeight / sourceHeight);
  canvas.style.width = `${Math.max(1, Math.floor(sourceWidth * scale))}px`;
  canvas.style.height = `${Math.max(1, Math.floor(sourceHeight * scale))}px`;
}

// 侧栏尺寸、窗口尺寸、设备旋转等变化时重新 contain。
if (window.ResizeObserver) {
  viewportObserver = new ResizeObserver(() => fitCanvasToViewport());
  viewportObserver.observe(viewport);
} else {
  window.addEventListener('resize', fitCanvasToViewport);
}
window.addEventListener('orientationchange', fitCanvasToViewport);

async function loadWasm() {
  if (!wasmReady) {
    wasmReady = import('/pkg/osu_beatmap_preview_wasm.js').then(async module => {
      await module.default();
      return module;
    });
  }
  return wasmReady;
}

async function fetchBeatmap(value) {
  const response = await fetch(`/resource/beatmap?bid=${encodeURIComponent(value)}`);
  if (!response.ok) throw new Error(await response.text());
  return new Uint8Array(await response.arrayBuffer());
}

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
  canvas.style.backgroundImage = '';
}

async function loadBackground(value) {
  const response = await fetch(`/resource/background?bid=${encodeURIComponent(value)}`);
  if (!response.ok) throw new Error(await response.text());
  const bitmap = await createImageBitmap(await response.blob());
  if (backgroundBitmap) backgroundBitmap.close();
  backgroundBitmap = bitmap;
  paintBackground();
}

async function loadAudioBlob(value) {
  const response = await fetch(`/resource/audio?bid=${encodeURIComponent(value)}`);
  if (!response.ok) throw new Error(await response.text());
  const blob = await response.blob();
  if (!blob.size) throw new Error('音频资源为空');
  const url = URL.createObjectURL(blob);
  audio.src = url;
  audio.dataset.objectUrl = url;
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

function updateControls() {
  seek.value = String(position);
  current.textContent = formatTime(position);
  total.textContent = formatTime(duration);
  play.textContent = playing ? '暂停' : '播放';
}

function render() {
  if (!session) return;
  try {
    session.render_number(absoluteTime());
    empty.textContent = '等待渲染';
    empty.hidden = true;
  } catch (error) {
    playing = false;
    audio.pause();
    cancelAnimationFrame(animation);
    empty.textContent = '渲染失败，请查看日志';
    empty.hidden = false;
    log($('#play-log'), `渲染失败：${errorText(error)}`);
  }
}

function syncAudioToPosition() {
  const time = absoluteTime();
  audioEnded = false;
  // HTMLMediaElement 的 currentTime 是异步 seek。短时间内不能用旧值覆盖新的进度。
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

function seekTo(nextPosition) {
  const resume = playing;
  playState(false);
  position = Math.min(duration, Math.max(0, nextPosition));
  syncAudioToPosition();
  render();
  updateControls();
  revealControls();
  if (resume) playState(true);
}

function playState(next) {
  playing = next;
  lastTick = 0;
  lastRenderTime = 0;
  hasRenderedFrame = false;
  cancelAnimationFrame(animation);
  audio.playbackRate = playbackRate();
  if (playing) {
    revealControls();
    syncAudioToPosition();
    if (absoluteTime() >= 0 && !audioEnded) {
      audio.play().catch(error => {
        if (!audioFailureLogged) {
          log($('#play-log'), `音频播放失败，继续播放画面：${errorText(error)}`);
          audioFailureLogged = true;
        }
      });
    }
    animation = requestAnimationFrame(tick);
  } else {
    audio.pause();
    keepControlsVisible();
  }
  updateControls();
}

function tick(now) {
  if (!playing) return;
  if (!lastTick) lastTick = now;
  const delta = now - lastTick;
  lastTick = now;
  if (absoluteTime() < 0 || audio.paused || audioEnded || now < ignoreAudioUntil) {
    position = Math.min(duration, position + delta * playbackRate());
    if (absoluteTime() >= 0 && audio.paused && !audioEnded) {
      syncAudioToPosition();
      audio.play().catch(error => {
        if (!audioFailureLogged) {
          log($('#play-log'), `音频播放失败，继续播放画面：${errorText(error)}`);
          audioFailureLogged = true;
        }
      });
    }
  } else {
    const audioPosition = audio.currentTime * 1000 - absoluteStart;
    // seek 或浏览器缓冲期间 currentTime 可能暂时回到旧值；只接受接近当前进度的时钟。
    if (Number.isFinite(audioPosition) && Math.abs(audioPosition - position) < 1500) {
      position = Math.min(duration, Math.max(0, audioPosition));
    } else {
      position = Math.min(duration, position + delta * playbackRate());
    }
  }

  // 始终按显示刷新率推进时钟，但只按所选帧率提交渲染。用目标帧间隔
  // 累加而不是直接用 `now` 覆盖，避免 144Hz 等显示器上 60FPS 被降到 48FPS。
  const frameInterval = 1000 / targetFps;
  if (!hasRenderedFrame) {
    hasRenderedFrame = true;
    lastRenderTime = now;
    render();
    updateControls();
    if (!playing) return;
  } else if (now - lastRenderTime >= frameInterval) {
    lastRenderTime += frameInterval * Math.floor((now - lastRenderTime) / frameInterval);
    render();
    updateControls();
    if (!playing) return;
  }

  if (position >= duration) {
    playState(false);
    return;
  }
  animation = requestAnimationFrame(tick);
}

async function loadPreview(event) {
  event.preventDefault();
  $('#load-log').textContent = '';
  try {
    bid = $('#bid').value.trim();
    if (!bid) throw new Error('请输入谱面 BID');
    const [wasm, bytes] = await Promise.all([loadWasm(), fetchBeatmap(bid)]);
    const resolution = selectedResolution();
    const options = {
      convert: $('#convert').value || undefined,
      // Mod 只通过播放页的可选控件设置，避免手写 token 与当前模式不匹配。
      mods: [],
      // WASM 会话和 core 的 RealtimeOptions 都使用这组输出宽高。
      width: resolution.width,
      height: resolution.height,
    };
    activeMods = options.mods.slice();
    session = await wasm.WebGpuSession.create(bytes, canvas, options);
    duration = Math.max(1, session.duration_ms_number());
    absoluteStart = session.absolute_start_ms_number();
    beatmapSpeed = session.beatmap_speed_number();
    position = 0;
    audioFailureLogged = false;
    audioEnded = false;
    seek.max = String(duration);
    $('#status').textContent = '加载背景和音频...';
    const [backgroundResult, audioResult] = await Promise.allSettled([
      loadBackground(bid),
      loadAudioBlob(bid),
    ]);
    if (backgroundResult.status === 'rejected') {
      log($('#play-log'), `背景加载失败，将使用黑色背景：${errorText(backgroundResult.reason)}`);
    }
    if (audioResult.status === 'rejected') {
      log($('#play-log'), `音频加载失败，将只播放画面：${errorText(audioResult.reason)}`);
    }
    canvas.width = session.width();
    canvas.height = session.height();
    syncAudioToPosition();
    $('#mode').textContent = session.mode().toUpperCase();
    buildModControls();
    loadPage.hidden = true;
    playPage.hidden = false;
    fitCanvasToViewport();
    $('#status').textContent = audioResult.status === 'fulfilled' ? '就绪' : '无音频';
    updateControls();
    render();
    // 加载完成后直接进入播放状态。浏览器可能因自动播放策略拒绝音频，
    // 但画面时钟仍由 requestAnimationFrame 继续推进。
    playState(true);
  } catch (error) {
    log($('#load-log'), error);
  }
}

const modOptions = {
  // README「Mod 支持」表：Standard / Taiko / Catch / Mania 的 GIF & MP4 支持项。
  standard: ['EZ', 'HR', 'HD', 'DA', 'TC', 'DT', 'HT'],
  taiko: ['EZ', 'HR', 'SW', 'CS', 'DT', 'HT'],
  catch: ['EZ', 'HR', 'DT', 'HT'],
  mania: ['CS', 'DT', 'HT', '1K', '2K', '3K', '4K', '5K', '6K', '7K', '8K', '9K', '10K', 'DS', 'IN', 'HO'],
};

// DA 需要参数；UI 暴露 AR/CS 两个可调参数，勾选 DA 后生成 DAAR<value>CS<value>。
const daNumber = value => {
  const number = Number(value);
  return Number.isFinite(number) ? String(number) : '0';
};
const activeDaToken = () => `DAAR${daNumber(daAr.value)}CS${daNumber(daCs.value)}`;
const updateDaLabels = () => {
  daArValue.textContent = Number(daAr.value).toFixed(1);
  daCsValue.textContent = Number(daCs.value).toFixed(1);
};

function buildModControls() {
  modControls.textContent = '';
  const options = modOptions[session.mode()] || [];
  for (const token of options) {
    const label = document.createElement('label');
    const input = document.createElement('input');
    input.type = 'checkbox';
    input.value = token;
    input.checked = activeMods.some(value => value.toUpperCase() === token);
    if (token === 'DA') input.title = 'Difficulty Adjust（可调 AR / CS）';
    input.addEventListener('change', updateMods);
    label.append(input, document.createTextNode(token));
    modControls.append(label);
  }
  daControls.hidden = !options.includes('DA');
  updateDaLabels();
}

function updateMods() {
  if (!session || modSwitching) return;
  const requested = [...modControls.querySelectorAll('input:checked')].map(input => input.value);
  const previous = activeMods;
  modSwitching = true;
  modControls.querySelectorAll('input').forEach(input => { input.disabled = true; });
  const submitted = requested
    .filter(token => token !== 'DA')
    .concat(requested.includes('DA') ? [activeDaToken()] : []);
  try {
    session.set_mods(submitted);
    activeMods = requested;
    duration = Math.max(1, session.duration_ms_number());
    absoluteStart = session.absolute_start_ms_number();
    beatmapSpeed = session.beatmap_speed_number();
    audio.playbackRate = playbackRate();
    position = Math.min(position, duration);
    seek.max = String(duration);
    syncAudioToPosition();
    render();
    updateControls();
    revealControls();
  } catch (error) {
    log($('#play-log'), `Mod 切换失败：${errorText(error)}`);
    modControls.querySelectorAll('input').forEach(input => {
      input.checked = previous.some(value => value.toUpperCase() === input.value);
    });
  } finally {
    modControls.querySelectorAll('input').forEach(input => { input.disabled = false; });
    modSwitching = false;
  }
}

function applyResolution() {
  if (!session) return;
  const { width, height } = selectedResolution();
  if (session.width() === width && session.height() === height) return;
  const previous = resolutionKeyFor(session.width(), session.height());
  const resume = playing;
  if (resume) playState(false);
  try {
    // session.resize 同时更新 WASM surface 和 core 的合成尺寸，否则场景会
    // 继续按旧宽高绘制并贴在新画布的左上角。
    session.resize(width, height);
    canvas.width = width;
    canvas.height = height;
    fitCanvasToViewport();
    paintBackground();
    render();
    updateControls();
    revealControls();
  } catch (error) {
    resolutionSelect.value = previous;
    log($('#play-log'), `分辨率切换失败：${errorText(error)}`);
  } finally {
    if (resume) playState(true);
  }
}

form.addEventListener('submit', loadPreview);
play.addEventListener('click', () => playState(!playing));
canvas.addEventListener('click', () => playState(!playing));
$('#minus').addEventListener('click', () => seekTo(position - 5000));
$('#plus').addEventListener('click', () => seekTo(position + 5000));
seek.addEventListener('input', () => seekTo(Number(seek.value)));
speed.addEventListener('change', () => {
  audio.playbackRate = playbackRate();
  revealControls();
});
fpsSelect.addEventListener('change', () => {
  targetFps = selectedFps();
  lastRenderTime = 0;
  hasRenderedFrame = false;
  if (!playPage.hidden) render();
  revealControls();
});
resolutionSelect.addEventListener('change', applyResolution);

// DA 的 AR/CS 滑杆：松开后自动勾选 DA 并重新应用 Mod。
daAr.addEventListener('input', updateDaLabels);
daCs.addEventListener('input', updateDaLabels);
daAr.addEventListener('change', () => {
  updateDaLabels();
  const daInput = [...modControls.querySelectorAll('input')].find(input => input.value === 'DA');
  if (daInput) daInput.checked = true;
  updateMods();
});
daCs.addEventListener('change', () => {
  updateDaLabels();
  const daInput = [...modControls.querySelectorAll('input')].find(input => input.value === 'DA');
  if (daInput) daInput.checked = true;
  updateMods();
});

// 鼠标移入视频区域或触摸时显示 HUD；移出后延迟隐藏。
viewport.addEventListener('mousemove', revealControls);
viewport.addEventListener('touchstart', revealControls, { passive: true });
viewport.addEventListener('mouseleave', () => scheduleControlsHide(700));
playerControls.addEventListener('mouseenter', () => clearTimeout(controlsHideTimer));
playerControls.addEventListener('mouseleave', () => scheduleControlsHide(900));
playerControls.addEventListener('focusin', keepControlsVisible);
playerControls.addEventListener('focusout', () => scheduleControlsHide(900));

audio.addEventListener('ended', () => {
  // 音频可能比谱面短，结束后继续用 requestAnimationFrame 驱动画面时钟。
  audioEnded = true;
  if (!playing) updateControls();
});
audio.addEventListener('loadedmetadata', () => {
  if (pendingAudioTime !== null && Number.isFinite(audio.duration)) {
    audio.currentTime = Math.min(pendingAudioTime, audio.duration);
    pendingAudioTime = null;
  }
});
audio.addEventListener('error', () => {
  const message = audio.error?.message || '音频无法解码或播放';
  $('#status').textContent = '音频错误';
  log($('#play-log'), `音频失败：${message}`);
});

$('#back').addEventListener('click', () => {
  playState(false);
  clearTimeout(controlsHideTimer);
  viewport.classList.remove('controls-visible');
  cancelAnimationFrame(animation);
  const objectUrl = audio.dataset.objectUrl;
  if (objectUrl) URL.revokeObjectURL(objectUrl);
  delete audio.dataset.objectUrl;
  audio.removeAttribute('src');
  audio.load();
  if (backgroundBitmap) {
    backgroundBitmap.close();
    backgroundBitmap = null;
  }
  canvas.style.backgroundImage = '';
  canvas.style.width = '';
  canvas.style.height = '';
  $('#play-log').textContent = '';
  session = null;
  playPage.hidden = true;
  loadPage.hidden = false;
});

const isEditableTarget = target =>
  target instanceof HTMLElement
  && (target.isContentEditable || ['INPUT', 'SELECT', 'TEXTAREA'].includes(target.tagName));

document.addEventListener('keydown', event => {
  if (playPage.hidden || isEditableTarget(event.target)) return;
  if (event.code === 'Space') {
    event.preventDefault();
    playState(!playing);
    return;
  }
  if (event.repeat) return;
  if (event.code === 'ArrowLeft') {
    event.preventDefault();
    seekTo(position - 5000);
    return;
  }
  if (event.code === 'ArrowRight') {
    event.preventDefault();
    seekTo(position + 5000);
  }
});

targetFps = selectedFps();
updateControls();
