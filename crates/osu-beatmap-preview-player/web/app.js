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
const current = $('#current');
const total = $('#total');
const empty = $('#empty');

let session = null;
let bid = '';
let playing = false;
let position = 0;
let duration = 1;
let absoluteStart = 0;
let beatmapSpeed = 1;
let lastTick = 0;
let animation = 0;
let wasmReady = null;
let pendingAudioTime = null;
let audioEnded = false;
let ignoreAudioUntil = 0;
let audioFailureLogged = false;
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
const log = (target, message) => { target.textContent += `${new Date().toLocaleTimeString()} ${errorText(message)}\n`; };
const playbackRate = () => Number(speed.value) * beatmapSpeed;
const absoluteTime = () => absoluteStart + position;

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

async function loadBackground(value) {
  const response = await fetch(`/resource/background?bid=${encodeURIComponent(value)}`);
  if (!response.ok) throw new Error(await response.text());
  const bitmap = await createImageBitmap(await response.blob());
  const decoder = document.createElement('canvas');
  decoder.width = session.width();
  decoder.height = session.height();
  const decoderContext = decoder.getContext('2d', { willReadFrequently: true });
  decoderContext.drawImage(bitmap, 0, 0, decoder.width, decoder.height);
  bitmap.close();
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

function loadAudio(value) {
  return new Promise((resolve, reject) => {
    let settled = false;
    const ready = () => {
      if (settled) return;
      settled = true;
      audio.removeEventListener('canplay', ready);
      audio.removeEventListener('loadedmetadata', ready);
      audio.removeEventListener('error', failed);
      resolve();
    };
    const failed = () => {
      if (settled) return;
      settled = true;
      audio.removeEventListener('canplay', ready);
      audio.removeEventListener('loadedmetadata', ready);
      audio.removeEventListener('error', failed);
      reject(new Error(audio.error?.message || '音频无法解码或播放'));
    };
    audio.addEventListener('canplay', ready, { once: true });
    audio.addEventListener('loadedmetadata', ready, { once: true });
    audio.addEventListener('error', failed, { once: true });
    audio.src = `/resource/audio?bid=${encodeURIComponent(value)}`;
    audio.load();
  });
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
  if (resume) playState(true);
}

function playState(next) {
  playing = next;
  lastTick = 0;
  audio.playbackRate = playbackRate();
  if (playing) {
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
  render();
  updateControls();
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
    const options = {
      convert: $('#convert').value || undefined,
      mods: $('#mods').value.split(/[\s,]+/).filter(Boolean),
      // 加载页中的 Canvas 处于 hidden 状态，不能从 clientWidth/clientHeight 取尺寸。
      width: 1280,
      height: 720,
    };
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
    loadPage.hidden = true;
    playPage.hidden = false;
    $('#status').textContent = audioResult.status === 'fulfilled' ? '就绪' : '无音频';
    updateControls();
    render();
  } catch (error) {
    log($('#load-log'), error);
  }
}

form.addEventListener('submit', loadPreview);
play.addEventListener('click', () => playState(!playing));
canvas.addEventListener('click', () => playState(!playing));
$('#minus').addEventListener('click', () => seekTo(position - 5000));
$('#plus').addEventListener('click', () => seekTo(position + 5000));
seek.addEventListener('input', () => seekTo(Number(seek.value)));
speed.addEventListener('change', () => { audio.playbackRate = playbackRate(); });
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
  cancelAnimationFrame(animation);
  const objectUrl = audio.dataset.objectUrl;
  if (objectUrl) URL.revokeObjectURL(objectUrl);
  delete audio.dataset.objectUrl;
  audio.removeAttribute('src');
  audio.load();
  canvas.style.backgroundImage = '';
  session = null;
  playPage.hidden = true;
  loadPage.hidden = false;
});
document.addEventListener('keydown', event => { if (event.code === 'Space' && !/INPUT|SELECT|TEXTAREA/.test(document.activeElement.tagName) && !playPage.hidden) { event.preventDefault(); playState(!playing); } });
updateControls();
