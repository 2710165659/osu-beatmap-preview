// 音频输出：AudioContext + AudioWorklet + SharedArrayBuffer 环形缓冲。
//
// 职责边界：WASM 会话负责「什么时候响、多大声」——事件时间轴、音乐、倍速换算与混音
// 都在里面（`pullAudio` 直接给出「音乐 + 打击音」的统一混音结果）；本文件只做搬运：
//
// - 每帧把 `session.pullAudio()` 的结果写进环形缓冲；
// - 音频线程按硬件时钟消费并回报「已读到哪」，这个位置转发回 `session.onAudioClock()`
//   ——它是画面时钟的锚点，音画同步的根基；
// - 输出流整体重置（seek、走散重对齐）由 `session.audioEpoch()` 的纪元变化驱动：
//   纪元一变就把环形读写指针一起归零，并通知音频线程从头重读。
//
// 位置统一用「相对帧号」（自上次重置起的采样帧），主线程写入与音频线程读取同一坐标系，
// 因此环形绕回不影响比较。协议见 `public/hitsound-worklet.js`：
// - 主线程 → 内核：`{ ring, control, mask, readFrame, playing, rate }`
// - 主线程 → 内核：`{ command: 'readFrame', value }`（重置读取指针）
// - 内核 → 主线程：`{ readPosition, frames, underrunFrames }`

import { createRingConfig, writeChunk } from './hitsound-core.js';

/** 一次最多向 WASM 要多少帧（48kHz 下约 85ms），限制单帧的混音与拷贝开销。 */
const MAX_PULL_FRAMES = 4096;

/**
 * 一条「音乐 + 打击音」的音频输出。
 *
 * 混音结果已经在 WASM 里按同一条时间轴合成，这里没有独立时钟：暂停、倍速、seek
 * 全部通过会话完成，输出只跟随。
 */
export class AudioOutput {
  /**
   * @param {object} options
   * @param {object} options.session WASM 会话（含混音与时钟）
   * @param {AudioContext} options.context 已创建的 AudioContext
   * @param {AudioWorkletNode} options.node 已连接的 worklet 节点
   * @param {Float32Array} options.ring 环形采样缓冲（交错立体声 f32）
   * @param {Int32Array} options.control 与音频线程共享的控制字
   * @param {number} options.mask 环形掩码
   */
  constructor({ session, context, node, ring, control, mask }) {
    this.session = session;
    this.context = context;
    this.node = node;
    this.ring = ring;
    this.control = control;
    this.mask = mask;
    /** 已写入环形的相对帧数（音频线程用同一坐标系比较）。 */
    this.writtenFrames = 0;
    /** 上一次看到的输出流纪元；变化表示整条流已重置。 */
    this.epoch = session.audioEpoch();
    /** 音频线程因为来不及混音而输出静音的累计帧数（诊断用）。 */
    this.underrunFrames = 0;
    this.playing = false;

    this.node.port.onmessage = (event) => {
      const data = event.data;
      if (!data) return;
      // 音频线程的消费位置就是「此刻听到的谱面位置」，转发给 WASM 锚定画面时钟。
      if (Number.isFinite(data.readPosition)) this.session.onAudioClock(data.readPosition);
      if (Number.isFinite(data.underrunFrames) && data.underrunFrames > 0) {
        this.underrunFrames += data.underrunFrames;
      }
    };
  }

  /** 让音频线程开始/停止消费；暂停时停止消费，保持两边位置一致。 */
  setPlaying(playing) {
    const next = Boolean(playing);
    if (this.playing === next) return;
    this.playing = next;
    this.node.port.postMessage({ playing: next });
  }

  /** 设置消费速率（总倍速 = 用户倍速 × 谱面变速，取 `session.rate()`）。 */
  setRate(rate) {
    if (!Number.isFinite(rate) || rate <= 0) return;
    this.node.port.postMessage({ rate });
  }

  /**
   * 补足环形缓冲：宿主每帧调用一次即可。
   *
   * 先混音后对账：`pullAudio` 内部可能因走散重置输出流，返回的那段数据属于新纪元，
   * 必须先归零写入指针再写，音频线程才读得到。
   */
  ensureBuffered() {
    const samples = this.session.pullAudio(MAX_PULL_FRAMES);
    const epoch = this.session.audioEpoch();
    if (epoch !== this.epoch) {
      this.epoch = epoch;
      this.writtenFrames = 0;
      Atomics.store(this.control, 0, 0);
      // 重置消息必须和写入指针一起落地，否则音频线程会把旧位置当新纪元读。
      this.node.port.postMessage({ command: 'readFrame', value: 0 });
    }
    const written = writeChunk(this.ring, this.mask, this.writtenFrames, samples);
    if (written > 0) {
      this.writtenFrames += written;
      // 控制字发布「已写入的相对帧数」；音频线程用同坐标系的读取位置与它比较。
      Atomics.store(this.control, 0, this.writtenFrames);
    }
  }

  close() {
    try {
      this.node.port.onmessage = null;
      this.node.disconnect();
    } catch (_) {
      // 关闭失败不影响页面继续运行。
    }
  }
}

/**
 * 创建音频上下文并加载播放内核。
 *
 * 采样率必须来自真实的 `AudioContext`，宿主拿它交给 WASM 的 `create`（混音输出
 * 采样率与设备一致，结果才能直接播放）。返回 `null` 表示当前环境不支持
 * （没有 AudioWorklet 或 SharedArrayBuffer），调用方应当退化成「只播放画面」。
 */
export async function createAudioContext(workletUrl) {
  if (typeof AudioContext === 'undefined' && typeof window === 'undefined') return null;
  const AudioContextClass = typeof AudioContext !== 'undefined' ? AudioContext : window?.webkitAudioContext;
  if (!AudioContextClass) return null;
  // 环形缓冲需要跨线程共享；没有 SharedArrayBuffer 时（例如缺少 COOP/COEP 头）
  // 直接放弃音频输出，而不是退化成主线程逐帧写缓冲区（那会引入新的音画不同步）。
  if (typeof SharedArrayBuffer === 'undefined' || typeof AudioWorkletNode === 'undefined') return null;

  let context;
  try {
    context = new AudioContextClass({ latencyHint: 'interactive' });
  } catch (_) {
    return null;
  }
  try {
    await context.audioWorklet.addModule(workletUrl);
  } catch (_) {
    await context.close().catch(() => {});
    return null;
  }
  return context;
}

/**
 * 在已创建的音频上下文上建立音频输出。
 *
 * 返回 `null` 表示节点创建失败；此时调用方应关闭上下文并退化成无声音。
 */
export function createAudioOutput({ session, context }) {
  if (!session || !context || typeof AudioWorkletNode === 'undefined') return null;
  if (typeof SharedArrayBuffer === 'undefined') return null;
  const config = createRingConfig(context.sampleRate);
  // 环形缓冲与控制字必须放进 SharedArrayBuffer，音频线程才能直接读。
  const ring = new Float32Array(new SharedArrayBuffer(config.length * 2 * 4));
  const control = new Int32Array(new SharedArrayBuffer(4));

  let node;
  try {
    node = new AudioWorkletNode(context, 'osu-hitsound', {
      numberOfInputs: 0,
      numberOfOutputs: 1,
      outputChannelCount: [2],
    });
  } catch (_) {
    return null;
  }
  node.connect(context.destination);
  node.port.postMessage({
    ring,
    control,
    mask: config.mask,
    playing: false,
    rate: session.rate(),
    readFrame: 0,
  });

  return new AudioOutput({ session, context, node, ring, control, mask: config.mask });
}
