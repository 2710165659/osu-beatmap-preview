// 网页端的打击音播放器。
//
// 职责边界（对应任务的架构要求）：
// - WASM 负责「什么时候播放哪个样本、多大声」：事件时间轴、倍速换算、混音都在里面；
// - 本文件只负责「把样本 PCM 送进 WASM」和「把混音结果按音频硬件时钟送出去」；
// - 下载仍走原有的 `/resource/*` 接口，音频线程只读共享环形缓冲区。
//
// 位置的说法统一如下：`gameMilliseconds` 是游戏时间轴上的毫秒（0 = 首个物件），
// WASM 的打击音位置用「谱面绝对时间」，两者相差 `absoluteStart`，由调用方传入。

import { samplePcmForWasm } from './hitsound-core.js';
import { createHitsoundStream } from './hitsound-stream.js';

/**
 * 把样本推给 WASM 的并发上限。
 *
 * 每个样本都要过一次 `setHitsoundSample`；并发太高会让主线程长时间被占用，
 * 太低又拉长准备时间，4 是一个折中。
 */
const SAMPLE_DECODE_CONCURRENCY = 4;

/**
 * 把解码出的 AudioBuffer 交给 WASM。
 *
 * 返回是否成功放入；放入失败只影响这一个音效，不影响其它样本。
 */
function pushSample(session, name, buffer, loopLength) {
  const { channels, samples } = samplePcmForWasm(buffer);
  if (!samples.length) return false;
  session.setHitsoundSample(name, channels, buffer.sampleRate, loopLength, samples);
  return true;
}

/** 从 WASM 取出上一批混音结果；WASM 未启用打击音时返回 null。 */
function takeMixedFrames(session) {
  const samples = session.takeHitsoundBuffer?.();
  if (!samples?.length) return null;
  return samples;
}

/** 用 AudioContext 解码一段 ogg；失败返回 null（按静音处理）。 */
async function decodeSample(context, bytes) {
  if (!bytes?.byteLength) return null;
  try {
    return await context.decodeAudioData(bytes.slice(0).buffer);
  } catch (_) {
    return null;
  }
}

export class HitsoundPlayer {
  /**
   * @param {object} options
   * @param {object} options.session WASM 会话（需要已调用 enableHitsound）
   * @param {AudioContext} options.context 已创建的 AudioContext
   * @param {AudioWorkletNode} options.node 已连接的 worklet 节点
   * @param {Float32Array} options.ring 环形采样缓冲（交错立体声 f32）
   * @param {Int32Array} options.control 与音频线程共享的控制字
   * @param {number} options.sampleRate 音频设备采样率
   * @param {number} options.absoluteStart 游戏时间 0 对应的谱面绝对时间
   */
  constructor(options) {
    this.session = options.session;
    this.context = options.context;
    this.node = options.node;
    this.absoluteStart = options.absoluteStart;
    this.sampleRate = options.sampleRate;
    this.active = false;
    this.rate = 1;

    // 坐标系与索引换算全部交给 HitsoundStream，播放器只负责 Web Audio 侧。
    this.stream = createHitsoundStream({
      sampleRate: options.sampleRate,
      ring: options.ring,
      control: options.control,
      render: (startMs, frames) => this.renderAt(startMs, frames),
      onPosition: (positionMs, restartVoices) => {
        if (restartVoices) this.session.seekHitsound(positionMs);
        else this.session.positionHitsound(positionMs);
      },
      post: (message) => this.post(message),
    });

    this.node.port.onmessage = (event) => {
      const data = event.data;
      if (!data) return;
      // 音频线程回报的是「自上次重置起已消费的帧数」与本次区间内读到静音的帧数，
      // 与写入位置同一坐标系；流靠前者把写入前沿保持在音频线程之前，后者只作诊断。
      this.stream.onReadFrame(data.readPosition, { underrunFrames: data.underrunFrames });
    };
  }

  /** 环形里还有多少帧没被消费（诊断用）。 */
  get bufferedAhead() {
    return this.stream.bufferedAhead;
  }

  /** 音频线程因为来不及混音而输出静音的累计帧数（诊断用；正常播放应当很小）。 */
  get underrunFrames() {
    return this.stream.underrunFrames;
  }

  /** 让内核开始/停止出声；暂停时停止消费，保持两边位置一致。 */
  setPlaying(playing) {
    const next = Boolean(playing);
    if (this.active === next) return;
    this.active = next;
    this.post({ playing: next });
  }

  /** 设置播放倍速；内核据此决定每个输出帧消耗多少游戏采样帧。 */
  setRate(rate) {
    if (!Number.isFinite(rate) || rate <= 0) return;
    this.rate = rate;
    this.post({ rate });
  }

  /**
   * 对齐位置：把「游戏时间」告诉 WASM，并让环形缓冲从对应采样帧重新开始。
   *
   * 只有明确的 seek 才允许重置环形；普通播放时让音频线程自己走时钟，
   * 避免每帧拨动造成抖动。
   */
  seekGameTime(gameMilliseconds, { seekMixer = false } = {}) {
    this.stream.reset(this.chartTime(gameMilliseconds), { restartVoices: seekMixer });
  }

  /** 游戏时间 → 谱面绝对时间（毫秒）。 */
  chartTime(gameMilliseconds) {
    const value = Number.isFinite(gameMilliseconds) ? gameMilliseconds : 0;
    return this.absoluteStart + value;
  }

  /** 补足环形缓冲：宿主每帧调用一次即可。 */
  ensureBuffered(chartTimeMs) {
    this.stream.ensureBuffered(chartTimeMs);
  }

  /** 让 WASM 从 `startMs` 起混出 `frames` 帧（位置由 `HitsoundStream` 负责对齐）。 */
  renderAt(startMs, frames) {
    // 锚点由流在重置时设定；这里只需保证 WASM 的位置与该段数据的起点一致。
    this.session.positionHitsound(startMs);
    const rendered = this.session.renderHitsound(frames);
    if (rendered <= 0) return null;
    // `takeHitsoundBuffer` 会取走缓冲，长度即本次渲染的帧数 × 2。
    return this.session.takeHitsoundBuffer?.() ?? null;
  }

  /**
   * 丢弃当前所有样本与已解析的数据。
   *
   * 切 Mod / 转谱后目标模式可能变化，需要的样本集合也随之变化；此时把 WASM 里的
   * 样本清空并重置环形缓冲，再重新加载，避免继续用旧模式的音效。
   */
  resetSamples() {
    this.session.resetHitsoundSamples();
    this.stream.reset(this.absoluteStart);
  }

  post(message) {
    this.node.port.postMessage(message);
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
 * 创建网页端的音频上下文并加载播放内核。
 *
 * 采样率必须来自真实的 `AudioContext`，所以宿主需要先拿到它，再告诉 WASM 用哪个
 * 采样率混音。返回 `null` 表示当前环境不支持（没有 AudioWorklet 或
 * SharedArrayBuffer），调用方应当退化成「只播画面与音乐」。
 */
export async function createHitsoundContext(workletUrl) {
  if (typeof AudioContext === 'undefined' && typeof window === 'undefined') return null;
  const AudioContextClass = typeof AudioContext !== 'undefined' ? AudioContext : window?.webkitAudioContext;
  if (!AudioContextClass) return null;
  // 环形缓冲需要跨线程共享；没有 SharedArrayBuffer 时（例如缺少 COOP/COEP 头）
  // 直接放弃打击音，而不是退化成主线程逐帧写缓冲区（那会引入新的音画不同步）。
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
 * 在已创建的音频上下文上建立打击音输出。
 *
 * 返回 `null` 表示节点创建失败；此时调用方应关闭上下文并退化成无打击音。
 */
export function createHitsoundOutput({ session, context, absoluteStart }) {
  if (!session || !context || typeof AudioWorkletNode === 'undefined') return null;
  if (typeof SharedArrayBuffer === 'undefined') return null;
  const stream = createHitsoundStream({
    sampleRate: context.sampleRate,
    ring: null,
    control: null,
    render: () => null,
    onPosition: () => {},
    post: () => {},
  });
  // 环形缓冲与控制字必须放进 SharedArrayBuffer，音频线程才能直接读。
  const ringBuffer = new SharedArrayBuffer(stream.ring.length * 4);
  const controlBuffer = new SharedArrayBuffer(4);
  const ring = new Float32Array(ringBuffer);
  const control = new Int32Array(controlBuffer);
  const mask = stream.ringFrames - 1;

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
  node.port.postMessage({ ring, control, mask, playing: false, rate: 1, readFrame: 0 });

  return new HitsoundPlayer({
    session,
    context,
    node,
    ring,
    control,
    sampleRate: stream.sampleRate,
    absoluteStart,
  });
}

/**
 * 加载全套打击音资源。
 *
 * 字节默认来自 WASM 内嵌的资源表（`hitsoundAsset`），宿主只负责用 Web Audio 解码成
 * PCM 再送回 WASM；因此不需要下载音效文件，也不存在「静态副本没同步」的问题。
 *
 * @param {object} options
 * @param {HitsoundPlayer} options.player
 * @param {string[]} options.names 需要加载的样本名
 * @param {(name: string) => Uint8Array|Promise<Uint8Array|null>|null} options.readAsset
 *   按名字取回样本字节。允许返回 Promise：后续接入「谱面自带音效」时，非内嵌的样本
 *   需要回落到后端去 OSZ 里取，读取就变成异步的。
 * @returns {Promise<number>} 成功放入的样本数
 */
export async function loadHitsoundSamples({ player, names, readAsset, onProgress }) {
  const queue = [];
  let loaded = 0;

  const run = async () => {
    while (queue.length) {
      const name = queue.shift();
      const bytes = await readAsset(name);
      const buffer = bytes?.length ? await decodeSample(player.context, bytes) : null;
      if (buffer) {
        // 整段循环的音效：滑行音需要首尾衔接，转盘旋转音同理。
        const loopLength = /sliderslide|spinnerspin/.test(name)
          ? Math.floor(buffer.duration * buffer.sampleRate)
          : 0;
        if (pushSample(player.session, name, buffer, loopLength)) loaded++;
      }
      onProgress?.(loaded, names.length);
    }
  };

  queue.push(...names);
  const workers = [];
  for (let index = 0; index < Math.min(SAMPLE_DECODE_CONCURRENCY, queue.length); index++) {
    workers.push(run());
  }
  await Promise.all(workers);
  // 全部样本放完后只重建一次事件时间轴：逐个样本重建会遍历整张谱面，样本多时很浪费。
  if (loaded > 0) player.session.rebuildHitsoundTimeline();
  return loaded;
}
