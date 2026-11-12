// Web 音频输出：共享缓冲 AudioWorklet → 消息传输 AudioWorklet → ScriptProcessor。
// WASM 统一负责混音、倍速与时间轴；各输出只搬运 PCM 并回报当前纪元的消费位置。
import { createRingConfig, readChunk, writeChunk } from './hitsound-core.js';

/** 一次最多向 WASM 要多少帧（48kHz 下约 85ms），限制单帧的混音与拷贝开销。 */
const MAX_PULL_FRAMES = 4096;
const workletContexts = new WeakSet();

/** 音乐与打击音共用的设备输出；暂停、倍速和 seek 都跟随 WASM 会话。 */
export class AudioOutput {
  constructor({ session, context, node, ring, control, mask, mode }) {
    this.session = session;
    this.context = context;
    this.node = node;
    this.ring = ring;
    this.control = control;
    this.mask = mask;
    this.mode = mode;
    this.writtenFrames = 0;
    this.readPosition = 0;
    this.epoch = session.audioEpoch();
    this.underrunFrames = 0;
    this.playing = false;
    this.rate = session.rate();
    this.pendingClocks = [];

    if (mode === 'script') {
      this.node.onaudioprocess = (event) => {
        const left = event.outputBuffer.getChannelData(0);
        const right = event.outputBuffer.getChannelData(1);
        left.fill(0);
        right.fill(0);
        if (!this.playing) return;
        // 旧浏览器可能不按绘制帧率调度页面，音频回调也负责补足预读。
        this.ensureBuffered();
        const result = readChunk(this.ring, this.mask, this.readPosition, this.writtenFrames, this.rate, left, right);
        this.readPosition = result.readPosition;
        this.underrunFrames += result.underrunFrames;
        // 回调填充的是未来播放的块，等设备时钟到达块末尾才报告，避免画面提前。
        this.pendingClocks.push({
          time: event.playbackTime + left.length / context.sampleRate,
          position: this.readPosition,
        });
      };
    } else {
      this.node.port.onmessage = ({ data }) => {
        if (!data || data.epoch !== this.epoch || data.epoch !== this.session.audioEpoch() || !this.playing) return;
        if (Number.isFinite(data.readPosition)) this.session.onAudioClock(data.readPosition);
        if (Number.isFinite(data.underrunFrames) && data.underrunFrames > 0) {
          this.underrunFrames += data.underrunFrames;
        }
      };
    }
  }

  setPlaying(playing) {
    const next = Boolean(playing);
    if (this.playing === next) return;
    this.playing = next;
    if (!next) this.pendingClocks.length = 0;
    this.node.port?.postMessage({ playing: next });
  }

  setRate(rate) {
    if (!Number.isFinite(rate) || rate <= 0) return;
    this.rate = rate;
    this.node.port?.postMessage({ rate });
  }

  /** 混音可能重置纪元，必须先对账再发布数据；旧纪元的时钟回报不能锚定新画面。 */
  ensureBuffered() {
    if (this.session.audioEpoch() === this.epoch) {
      let clock;
      while (this.pendingClocks[0]?.time <= this.context.currentTime) clock = this.pendingClocks.shift();
      if (clock) this.session.onAudioClock(clock.position);
    }
    const samples = this.session.pullAudio(MAX_PULL_FRAMES);
    const epoch = this.session.audioEpoch();
    if (epoch !== this.epoch) {
      this.epoch = epoch;
      this.writtenFrames = 0;
      this.readPosition = 0;
      this.pendingClocks.length = 0;
      if (this.control) Atomics.store(this.control, 0, 0);
      this.node.port?.postMessage({ command: 'readFrame', value: 0, epoch });
    }
    if (this.mode === 'message') {
      if (samples.length) {
        // WASM 返回值可能引用其内存；传输前取得独立所有权，避免分离 WASM 的 ArrayBuffer。
        const owned = samples.slice();
        this.node.port.postMessage({ samples: owned }, [owned.buffer]);
      }
    } else {
      this.writtenFrames += writeChunk(this.ring, this.mask, this.writtenFrames, samples);
      if (this.control) Atomics.store(this.control, 0, this.writtenFrames);
    }
  }

  close() {
    this.playing = false;
    this.pendingClocks.length = 0;
    this.node.onaudioprocess = null;
    if (this.node.port) {
      this.node.port.onmessage = null;
      this.node.port.close();
    }
    try {
      this.node.disconnect();
    } catch (_) {
      // 关闭失败不影响页面继续运行。
    }
  }
}

/** 创建实际设备采样率的上下文；worklet 加载失败时保留上下文供兼容输出使用。 */
export async function createAudioContext(workletUrl) {
  const AudioContextClass = globalThis.AudioContext ?? globalThis.webkitAudioContext;
  if (!AudioContextClass) return null;
  let context;
  try {
    context = new AudioContextClass({ latencyHint: 'interactive' });
  } catch (_) {
    return null;
  }
  if (context.audioWorklet && typeof AudioWorkletNode !== 'undefined') {
    try {
      await context.audioWorklet.addModule(workletUrl);
      workletContexts.add(context);
    } catch (_) {
      // 非安全上下文、策略限制或模块加载失败时继续尝试主线程输出。
    }
  }
  return context;
}

/** 依次尝试可用输出，只有所有设备节点都无法创建时才返回 null。 */
export function createAudioOutput({ session, context }) {
  if (!session || !context) return null;
  const config = createRingConfig(context.sampleRate);
  if (workletContexts.has(context)) {
    for (const mode of ['shared', 'message']) {
      if (mode === 'shared' && typeof SharedArrayBuffer === 'undefined') continue;
      let node;
      try {
        const ring = mode === 'shared' ? new Float32Array(new SharedArrayBuffer(config.length * 2 * 4)) : null;
        const control = mode === 'shared' ? new Int32Array(new SharedArrayBuffer(4)) : null;
        node = new AudioWorkletNode(context, 'osu-hitsound', {
          numberOfInputs: 0,
          numberOfOutputs: 1,
          outputChannelCount: [2],
        });
        node.port.postMessage({
          ring,
          control,
          ringFrames: mode === 'message' ? config.length : 0,
          mask: config.mask,
          playing: false,
          rate: session.rate(),
          epoch: session.audioEpoch(),
        });
        node.connect(context.destination);
        return new AudioOutput({ session, context, node, ring, control, mask: config.mask, mode });
      } catch (_) {
        node?.port.close();
        node?.disconnect();
      }
    }
  }
  let node;
  try {
    // ScriptProcessor 已弃用，仅用于无法建立 AudioWorklet 的环境。
    node = context.createScriptProcessor(1024, 0, 2);
    const ring = new Float32Array(config.length * 2);
    const output = new AudioOutput({ session, context, node, ring, mask: config.mask, mode: 'script' });
    node.connect(context.destination);
    return output;
  } catch (_) {
    if (node) node.onaudioprocess = null;
    node?.disconnect();
    return null;
  }
}
