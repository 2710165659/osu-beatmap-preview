const setup = document.querySelector('#setup');
const player = document.querySelector('#player');
const form = document.querySelector('#load-form');
const frame = document.querySelector('#frame');
const frameStatus = document.querySelector('#frame-status');
const audio = document.querySelector('#audio');
const seek = document.querySelector('#seek');
const playButton = document.querySelector('#play');
const currentTime = document.querySelector('#current-time');
const totalTime = document.querySelector('#total-time');
const speed = document.querySelector('#speed');
const logs = [];

let state = null;
let playing = false;
let loopRunning = false;
let renderGeneration = 0;
let playbackGeneration = 0;
let seekTimer = 0;

const formatTime = ms => {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = seconds % 60;
  return h
    ? `${String(h).padStart(2, '0')}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`
    : `${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
};

function logError(message) {
  logs.push(message);
  document.querySelectorAll('.log').forEach(log => { log.hidden = false; });
  document.querySelectorAll('.log pre').forEach(pre => {
    pre.textContent = logs.map((item, index) => `${index + 1}: ${item}`).join('\n');
  });
}

function showSetupError(message) {
  const box = document.querySelector('#setup-error');
  box.hidden = false;
  box.textContent = message;
  logError(message);
}

function playbackDuration() {
  if (!state) return 0;
  return Math.max(0, state.last_object_ms - state.absolute_start_ms);
}

function absoluteTime() {
  return state.absolute_start_ms + Number(seek.value);
}

function updateTime() {
  const elapsed = Number(seek.value);
  currentTime.textContent = formatTime(elapsed);
  totalTime.textContent = formatTime(playbackDuration());
  seek.setAttribute('aria-valuetext', `${formatTime(elapsed)} / ${formatTime(playbackDuration())}`);
}

function updatePlayButton() {
  playButton.textContent = playing ? 'Ⅱ' : '▶';
  playButton.setAttribute('aria-label', playing ? '暂停' : '播放');
}

function audioTimeForSeek() {
  // 音频文件从谱面绝对时间 0 开始，起点前的部分由画面时钟补齐静音。
  return Math.max(0, absoluteTime()) / 1000;
}

function syncAudioToSeek() {
  if (!state?.has_audio) return;
  audio.currentTime = audioTimeForSeek();
}

function syncFromAudio() {
  if (!state?.has_audio || !playing || absoluteTime() < 0) return;
  const absolute = Math.max(0, audio.currentTime * 1000);
  const elapsed = Math.max(0, absolute - state.absolute_start_ms);
  seek.value = Math.min(Number(seek.max), Math.round(elapsed));
  updateTime();
}

async function renderFrame() {
  if (!state) return;
  const generation = ++renderGeneration;
  try {
    const response = await fetch(`/api/frame?time=${absoluteTime()}&t=${Date.now()}`);
    if (!response.ok) {
      const body = await response.json().catch(() => ({}));
      throw new Error(body.error || '帧渲染失败');
    }
    const blob = await response.blob();
    // 用户拖动后，丢弃已经过时的响应，避免旧帧覆盖新帧。
    if (generation !== renderGeneration) return;
    const old = frame.src;
    frame.src = URL.createObjectURL(blob);
    if (old.startsWith('blob:')) URL.revokeObjectURL(old);
    frameStatus.hidden = true;
  } catch (error) {
    frameStatus.hidden = false;
    frameStatus.textContent = error.message;
    logError(error.message);
    setPlaying(false);
  }
}

function setPlaying(value) {
  if (value) clearTimeout(seekTimer);
  playbackGeneration++;
  playing = Boolean(value);
  if (state?.has_audio) {
    audio.playbackRate = Number(speed.value) * state.beatmap_speed;
    if (playing) {
      syncAudioToSeek();
      // 起点在 0 之前时先由画面时钟播放，音频等绝对时间进入 0 后再启动。
      if (absoluteTime() >= 0) {
        audio.play().catch(error => logError(`音频播放失败：${error.message}`));
      } else {
        audio.pause();
      }
    } else {
      audio.pause();
    }
  }
  updatePlayButton();
  if (playing) playbackLoop();
}

async function playbackLoop() {
  if (loopRunning) return;
  loopRunning = true;
  const generation = playbackGeneration;
  let previous = performance.now();
  try {
    while (playing && state && generation === playbackGeneration) {
      const frameStarted = performance.now();
      const now = performance.now();
      const delta = now - previous;
      previous = now;

      if (state.has_audio) {
        if (absoluteTime() < 0) {
          seek.value = Math.min(Number(seek.max), Number(seek.value) + delta * Number(speed.value) * state.beatmap_speed);
          if (absoluteTime() >= 0) {
            syncAudioToSeek();
            audio.play().catch(error => logError(`音频播放失败：${error.message}`));
          }
        } else {
          syncFromAudio();
        }
      } else {
        seek.value = Math.min(Number(seek.max), Number(seek.value) + delta * Number(speed.value) * state.beatmap_speed);
      }
      updateTime();
      await renderFrame();
      if (Number(seek.value) >= Number(seek.max)) {
        setPlaying(false);
        break;
      }
      // 按渲染配置限速，渲染耗时过长时自然降速，不再占满请求队列。
      const frameWait = Math.max(0, 1000 / (state.target_fps || 30) - (performance.now() - frameStarted));
      await new Promise(resolve => setTimeout(resolve, frameWait));
    }
  } finally {
    loopRunning = false;
  }
}

async function seekTo(value) {
  seek.value = Math.max(0, Math.min(Number(seek.max), Number(value)));
  setPlaying(false);
  renderGeneration++;
  syncAudioToSeek();
  updateTime();
  await renderFrame();
}

form.addEventListener('submit', async event => {
  event.preventDefault();
  const button = form.querySelector('button');
  button.disabled = true;
  button.querySelector('span').textContent = '加载中...';
  try {
    const mods = document.querySelector('#mods').value.split(/[\s,]+/).filter(Boolean);
    const rawScale = document.querySelector('#scale').value.trim();
    const response = await fetch('/api/load', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        bid: document.querySelector('#bid').value.trim(),
        mods,
        convert: document.querySelector('#convert').value || null,
        scale: rawScale ? Number(rawScale) : null,
        no_cache: document.querySelector('#no-cache').checked,
      }),
    });
    const data = await response.json();
    if (!response.ok) throw new Error(data.error || '加载失败');
    state = data;
    seek.max = Math.max(1, playbackDuration());
    seek.value = 0;
    document.querySelector('#mode-chip').textContent = data.mode.toUpperCase();
    updateTime();
    setup.hidden = true;
    player.hidden = false;
    audio.src = data.has_audio ? '/api/audio' : '';
    frameStatus.hidden = false;
    frameStatus.textContent = '渲染首帧...';
    await renderFrame();
  } catch (error) {
    showSetupError(error.message);
  } finally {
    button.disabled = false;
    button.querySelector('span').textContent = '加载预览';
  }
});

playButton.addEventListener('click', () => setPlaying(!playing));
document.querySelector('.stage').addEventListener('click', () => {
  if (state) setPlaying(!playing);
});
document.querySelector('#back5').addEventListener('click', () => seekTo(Number(seek.value) - 5000));
document.querySelector('#forward5').addEventListener('click', () => seekTo(Number(seek.value) + 5000));
document.querySelector('#back').addEventListener('click', () => {
  setPlaying(false);
  renderGeneration++;
  audio.pause();
  audio.removeAttribute('src');
  audio.load();
  player.hidden = true;
  setup.hidden = false;
});
seek.addEventListener('input', () => {
  // 拖动时只渲染最后一次输入，避免请求队列堆积后把进度拉回旧位置。
  setPlaying(false);
  clearTimeout(seekTimer);
  const value = seek.value;
  seekTimer = setTimeout(() => seekTo(value), 50);
  updateTime();
});
speed.addEventListener('change', () => {
  if (state?.has_audio) audio.playbackRate = Number(speed.value) * state.beatmap_speed;
});
audio.addEventListener('timeupdate', syncFromAudio);
audio.addEventListener('ended', () => setPlaying(false));
document.addEventListener('keydown', event => {
  if (event.code === 'Space' && !['INPUT', 'SELECT', 'TEXTAREA'].includes(document.activeElement.tagName) && !player.hidden) {
    event.preventDefault();
    setPlaying(!playing);
  }
});
