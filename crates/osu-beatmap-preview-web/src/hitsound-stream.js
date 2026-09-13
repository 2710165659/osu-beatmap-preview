// 打击音输出流：把 WASM 混出的 PCM 送进与音频线程共享的环形缓冲。
//
// 这个类不碰 Web Audio API，只处理坐标系与索引：环形读写指针、控制字、解析窗口。
// 音画不同步最容易出在这里，因此有三条硬约束：
//
// 1. **坐标系**：音频线程只认识「相对帧号」（自上次重置起写了/读了多少帧），因此
//    控制字必须发布相对帧数（`writtenFrames`），不能发布绝对帧号——否则播放绕回
//    一圈后数据会被判成「未就绪」，整段静音。
// 2. **锚点**：`resolvedFrom` 是混音位置的**锚点帧**，只在重置/重新对齐时改变。
//    每帧都用当前时间调用 WASM 的 `positionHitsound` 会把锚点拽着走，导致
//    已经混合好的那批数据对应的时间被改写，打击音整体错位。
// 3. **预读必须领先音频线程**：填充目标由「画面时钟」和「音频线程已读位置」共同
//    决定。音频线程由音频硬件时钟驱动，通常比画面时钟（HTML 音频元素）早几十毫秒；
//    只按画面时钟预读时它会一路追到写入前沿，读到的全是静音——这正是「打击音时有时
//    无、拖动进度条后完全消失」的根因。

import { createRingConfig, msToFrames } from './hitsound-core.js';

/** 一次最多补多少帧，避免长时间后台后一次性写入巨量数据。 */
const MAX_FILL_FRAMES = 4096;

/** 毫秒换算成至少 1 帧，避免极短时长在低采样率下被截成 0。 */
const framesFor = (milliseconds, sampleRate) =>
  Math.max(1, Math.round(msToFrames(milliseconds, sampleRate)));

/**
 * 环形缓冲里预读多少毫秒。
 *
 * 音效从写入到被音频线程取走之间必须留出余量，余量要覆盖：主线程每帧补一次
 * （60Hz 约 17ms）、音频线程每 512 帧（48kHz 下约 11ms）才回报一次位置、以及主线程
 * 偶发卡顿。取 170ms 能盖住这些抖动，在 2 秒的环形里也很宽裕。
 *
 * 预读不会让声音滞后：环形里第 r 帧对应的谱面时间由重置时的锚点唯一确定，音频线程
 * 读到第 r 帧就播那个时间，与写入时刻无关；预读只是让数据更早备好。
 */
export const LOOKAHEAD_MS = 170;

/**
 * 锚点与目标位置之间允许的漂移（毫秒）；超出后重新对齐一次。
 *
 * 锚点跟着目标推进，正常情况下漂移只有调用间隔那一帧的量；容差留大一些是为了
 * 吸收倍速与主线程抖动，避免频繁清空正在播放的声音（对齐本身是一次轻微中断）。
 */
export const RESEEK_TOLERANCE_MS = 250;

/**
 * 锚点每次最多推进多少毫秒。
 *
 * 锚点必须跟着播放前进，否则写入位置会一路领先于音频线程的读取位置，
 * 最终绕回环形并覆盖尚未播放的数据（表现就是打击音整体错位或丢失）。
 * 取 250ms 既跟得上播放，又不会因为主线程偶发卡顿而跳过成片的事件。
 */
const MAX_ANCHOR_ADVANCE_MS = 250;

/**
 * 音频线程的读取位置最多可以把填充目标提前多少毫秒。
 *
 * 两条时钟的输出缓冲不同，音频线程稳定地领先画面时钟几十毫秒是正常的；但一次异常的
 * 位置报告（例如重置消息还没落地时回报的旧位置）不该驱动出巨量混音，因此设上限。
 * 真正的走散交给上面的重新对齐处理。
 */
const MAX_CONSUMER_LEAD_MS = 250;

export class HitsoundStream {
  /**
   * @param {object} options
   * @param {number} options.sampleRate 音频设备采样率
   * @param {Float32Array} options.ring 环形缓冲（交错立体声 f32，长度 = 帧数 × 2）
   * @param {Int32Array} options.control 与音频线程共享的控制字
   * @param {(startMs: number, frames: number) => Float32Array|null} options.render
   *   让 WASM 从 `startMs` 起混出 `frames` 帧；返回 null 表示不可用
   * @param {(positionMs: number, reset: boolean) => void} options.onPosition
   *   把位置告诉 WASM；`reset = true` 表示应当清空正在播放的声音
   * @param {(message: object) => void} options.post 向音频线程发消息
   */
  constructor({ sampleRate, ring, control, render, onPosition, post }) {
    this.sampleRate = sampleRate;
    this.ring = ring;
    this.control = control;
    this.render = render;
    this.onPosition = onPosition;
    this.post = post;

    /** 混音锚点（绝对帧号）：`resolvedFrom + resolvedFrames` 是下一个待混音的帧。 */
    this.resolvedFrom = 0;
    /** 从锚点起已经写进环形的帧数。 */
    this.resolvedFrames = 0;
    /** 主线程已写入的帧数（相对计数，与音频线程同一坐标系）。 */
    this.writtenFrames = 0;
    /** 上一次重置时锚点对应的绝对帧号：相对帧号 r 对应的谱面帧就是它 + r。 */
    this.baseFrame = 0;
    /** 音频线程已消费的帧数（相对计数，与 `writtenFrames` 同一坐标系）。 */
    this.consumedFrames = 0;
    /** 音频线程因为来不及混音而输出静音的累计帧数（诊断用）。 */
    this.underrunFrames = 0;

    // 时长常量按采样率换算成帧，保证不同设备的预读时长一致。
    this.lookaheadFrames = framesFor(LOOKAHEAD_MS, sampleRate);
    this.reseekToleranceFrames = framesFor(RESEEK_TOLERANCE_MS, sampleRate);
    this.maxAnchorAdvanceFrames = framesFor(MAX_ANCHOR_ADVANCE_MS, sampleRate);
    this.maxConsumerLeadFrames = framesFor(MAX_CONSUMER_LEAD_MS, sampleRate);
  }

  get ringFrames() {
    return this.ring.length / 2;
  }

  /**
   * 收到音频线程回报的读取位置（自上次重置起已消费的帧数）。
   *
   * 它可能落在写入前沿之后：音频线程超前后读到的是静音，那些帧并没有真的取到数据，
   * 不能算作「已经补到那儿了」，因此夹到写入范围内。夹住之后填充目标不会因为一次
   * 滞后或异常的报告而跑到很远的未来。
   *
   * @param {number} readPosition 已消费的相对帧数
   * @param {object} [options]
   * @param {number} [options.underrunFrames] 本次回报区间内读到静音的帧数（诊断用）
   */
  onReadFrame(readPosition, { underrunFrames = 0 } = {}) {
    if (Number.isFinite(readPosition) && readPosition > 0) {
      const clamped = Math.min(Math.floor(readPosition), this.writtenFrames);
      this.consumedFrames = Math.max(this.consumedFrames, clamped);
    }
    if (Number.isFinite(underrunFrames) && underrunFrames > 0) {
      this.underrunFrames += underrunFrames;
    }
  }

  /** 环形里已经写好、音频线程还没取走的帧数。 */
  get bufferedAhead() {
    return Math.max(0, Math.min(this.writtenFrames - this.consumedFrames, this.ringFrames));
  }

  /**
   * 重置整条流，并把锚点对齐到 `chartTimeMs`。
   *
   * 覆盖旧数据或 seek 时必须一起清零计数器：控制字与音频线程的读取指针都基于
   * 「相对帧数」，两边基准必须同时切换。
   */
  reset(chartTimeMs, { restartVoices = false } = {}) {
    this.writtenFrames = 0;
    this.consumedFrames = 0;
    this.resolvedFrames = 0;
    const frame = Math.max(0, Math.floor(msToFrames(chartTimeMs, this.sampleRate)));
    this.resolvedFrom = frame;
    this.baseFrame = frame;
    this.onPosition(this.positionMs, restartVoices);
    Atomics.store(this.control, 0, 0);
    // 必须同时把音频线程的读取指针拨回 0：向后 seek 之后它可能还停在旧位置，
    // 那边已经超过新的写入位置，会被判成「数据就绪」而读到垃圾数据。
    this.post({ command: 'readFrame', value: 0 });
  }

  /** 锚点对应的谱面时间（毫秒）。 */
  get positionMs() {
    return (this.resolvedFrom * 1000) / this.sampleRate;
  }

  /** 下一个待混音的谱面时间（毫秒）。 */
  get nextRenderMs() {
    return ((this.resolvedFrom + this.resolvedFrames) * 1000) / this.sampleRate;
  }

  /**
   * 让锚点跟着播放前进，把已经写入环形的部分「结算」掉。
   *
   * 不推进锚点就会让写入位置一路领先音频线程的读取位置，最终绕回环形覆盖还没播的
   * 数据——这正是打击音错位的根因。推进量限制在 [`MAX_ANCHOR_ADVANCE_MS`] 以内，
   * 避免主线程偶发卡顿导致成片事件被跳过。
   */
  advanceAnchor(targetFrame) {
    if (targetFrame <= this.resolvedFrom) return;
    const advance = Math.min(
      targetFrame - this.resolvedFrom,
      this.maxAnchorAdvanceFrames,
      this.resolvedFrames,
    );
    if (advance <= 0) return;
    this.resolvedFrom += advance;
    this.resolvedFrames -= advance;
    // 只移动位置，不清空正在播放的声音。
    this.onPosition(this.positionMs, false);
  }

  /**
   * 补足环形缓冲：宿主每帧调用一次即可。
   *
   * 填充目标取「画面时钟 + 预读」与「音频线程已读位置 + 预读」中更靠后的那个：
   * 前者保证内容按谱面时间轴连续渲染，后者保证音频线程永远有数据可读。
   *
   * @param {number} chartTimeMs 当前谱面绝对时间（毫秒）
   */
  ensureBuffered(chartTimeMs) {
    if (!Number.isFinite(chartTimeMs)) return;
    const target = Math.floor(msToFrames(chartTimeMs, this.sampleRate));
    const drift = target - this.resolvedFrom;

    // 漂移过大（seek、长时间后台）时重新对齐；对齐会清空正在播放的声音。
    if (drift < -this.reseekToleranceFrames || drift > this.reseekToleranceFrames) {
      this.reset(chartTimeMs);
    } else {
      this.advanceAnchor(target);
    }

    const pictureEnd = target + this.lookaheadFrames;
    const consumerEnd = this.baseFrame + this.consumedFrames + this.lookaheadFrames;
    const wantedEnd = Math.max(pictureEnd, Math.min(consumerEnd, pictureEnd + this.maxConsumerLeadFrames));

    // 相对帧号 r 对应的谱面帧 = baseFrame + r，写入前沿即 baseFrame + writtenFrames；
    // 它等价于 resolvedFrom + resolvedFrames（锚点释放只在这两个量之间搬运）。
    let remaining = Math.max(0, wantedEnd - (this.baseFrame + this.writtenFrames));
    while (remaining > 0) {
      const frames = Math.min(MAX_FILL_FRAMES, remaining);
      const startMs = this.nextRenderMs;
      const samples = this.render(startMs, frames);
      if (!samples?.length) break;
      const produced = Math.min(frames, Math.floor(samples.length / 2));
      if (produced <= 0) break;
      // 这段数据对应 `[写入前沿, 写入前沿 + produced)` 这段相对帧号，紧接在已写数据之后。
      this.write(samples, produced, this.writtenFrames);
      this.resolvedFrames += produced;
      remaining = Math.max(0, wantedEnd - (this.baseFrame + this.writtenFrames));
    }
  }

  /**
   * 把一段交错立体声写进环形缓冲。
   *
   * 写入位置只增不减，环形下标用取模得到；一旦写入前沿比音频线程领先超过一整圈，
   * 最旧的数据会被覆盖（音频线程用 `writtenFrames - ringFrames` 自己判断哪些数据
   * 已经不可读）。正常的预读窗口远小于一圈，只有主线程长时间卡顿时才会走到这一步。
   *
   * @param {Float32Array} samples 交错立体声 PCM
   * @param {number} frames 实际要写入的采样帧数
   * @param {number} atRelativeFrame 该段第 0 帧在相对坐标里的位置（默认接在已写数据之后）
   */
  write(samples, frames = Math.floor(samples.length / 2), atRelativeFrame = this.writtenFrames) {
    const ringFrames = this.ringFrames;
    const count = Math.min(frames, ringFrames, Math.floor(samples.length / 2));
    if (count <= 0) return;
    const start = Math.max(0, atRelativeFrame);
    for (let index = 0; index < count; index++) {
      const slot = ((start + index) % ringFrames) * 2;
      this.ring[slot] = samples[index * 2];
      this.ring[slot + 1] = samples[index * 2 + 1];
    }
    this.writtenFrames = Math.max(this.writtenFrames, start + count);
    // 控制字发布「已写入的相对帧数」；音频线程用同坐标系的读取位置与它比较。
    Atomics.store(this.control, 0, this.writtenFrames);
  }
}

/**
 * 创建一套与音频线程共享的打击音输出流。
 *
 * 不涉及 Web Audio API；调用方提供 `ring`/`control`（通常是 `SharedArrayBuffer` 视图）
 * 与三个回调。
 */
export function createHitsoundStream({ sampleRate, ring, control, render, onPosition, post }) {
  const config = createRingConfig(sampleRate);
  return new HitsoundStream({
    sampleRate: config.sampleRate,
    ring: ring ?? new Float32Array(config.length * 2),
    control: control ?? new Int32Array(1),
    render,
    onPosition,
    post: post ?? (() => {}),
  });
}
