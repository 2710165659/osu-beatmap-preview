// 打击音播放内核（AudioWorklet）。
//
// 它只做一件事：按音频硬件时钟从共享环形缓冲区里取 PCM 输出，并把「已经播到哪个
// 采样帧」通过消息回报给主线程。混音、事件时间轴、倍速换算的权威实现在 WASM 里，
// 这里不做任何时间判断，因此不存在两处时钟互相追的问题。
//
// 协议与 `src/hitsound.js` 中的 `createHitsoundOutput()` 一致：
// - 主线程 → 内核：`{ ring, control, mask, readFrame, playing, rate }`
// - 主线程 → 内核：`{ command: 'readFrame', value }`（seek / 重新对齐）
// - 内核 → 主线程：`{ readPosition, frames, underrunFrames }`
//
// 位置全部使用「相对帧号」：自上次重置（`command: 'readFrame'`）起读了多少帧。
// 主线程的写入位置用同一坐标系，因此两边的比较不受环形绕回影响。
// `readPosition` 是主线程判断「还要往前补多少」的依据，它必须按固定间距回报——
// 回报太稀，主线程就来不及把写入前沿保持在音频线程之前，声音会被读成静音；
// `underrunFrames` 则是「这段时间里有多少帧因为来不及混音而只能输出静音」。

/**
 * 位置回报间隔（帧）。
 *
 * 512 帧在 48kHz 下约 11ms：主线程每帧都会补缓冲，11ms 的粒度足以让它把预读窗口
 * 维持住；再稀就会留下静音窗口，再密则是无意义的跨线程流量。
 */
const REPORT_INTERVAL_FRAMES = 512;

class HitsoundProcessor extends AudioWorkletProcessor {
  constructor() {
    super();
    this.ring = null;
    this.control = null;
    this.mask = 0;
    // 读取位置保留为不回绕的相对帧号；只有写入数组下标才取环形掩码。
    // 这样写入超过一圈后仍能判断数据是否已被覆盖，避免把旧数据当成当前音效。
    this.readPosition = 0;
    this.playing = false;
    // 每个输出帧消耗的游戏采样帧数（= 播放倍速）。倍速播放时打击音随之变快，
    // 与音乐倍速保持一致。
    this.rate = 1;
    /** 自上次回报以来经过的输出帧数，用于按固定间距回报位置。 */
    this.reportedFrames = 0;
    /** 自上次回报以来读到静音（欠载）的帧数，供主线程诊断。 */
    this.starvedFrames = 0;
    this.port.onmessage = (event) => {
      const data = event.data;
      if (!data) return;
      if (data.command === 'readFrame' && Number.isFinite(data.value)) {
        this.readPosition = data.value;
        this.reportedFrames = 0;
        this.starvedFrames = 0;
        return;
      }
      if (data.ring) this.ring = data.ring;
      if (data.control) this.control = data.control;
      if (typeof data.mask === 'number') this.mask = data.mask;
      if (Number.isFinite(data.readFrame)) this.readPosition = data.readFrame;
      if (typeof data.playing === 'boolean') {
        this.playing = data.playing;
        this.reportedFrames = 0;
        this.starvedFrames = 0;
      }
      if (Number.isFinite(data.rate) && data.rate > 0) this.rate = data.rate;
    };
  }

  process(_inputs, outputs) {
    const output = outputs[0];
    const frames = output[0].length;
    const left = output[0];
    const right = output.length > 1 ? output[1] : null;

    if (!this.playing || !this.ring) {
      left.fill(0);
      if (right) right.fill(0);
      return true;
    }

    const ringFrames = this.ring.length / 2;
    const mask = this.mask;
    let cursor = this.readPosition;
    const writtenFrames = Atomics.load(this.control, 0);
    const oldestReadable = Math.max(0, writtenFrames - ringFrames);

    for (let index = 0; index < frames; index++) {
      const base = Math.floor(cursor);
      const fraction = cursor - base;
      // 控制字和读取位置都是不回绕的相对帧号；读取位置落后太多表示数据已被覆盖，
      // 超前则表示主线程还没来得及混音（欠载），这两种情况都输出静音。
      if (base < oldestReadable || base >= writtenFrames) {
        left[index] = 0;
        if (right) right[index] = 0;
        this.starvedFrames++;
      } else {
        const first = (base & mask) * 2;
        const second = ((base + 1) & mask) * 2;
        const l = this.ring[first] + (this.ring[second] - this.ring[first]) * fraction;
        left[index] = l;
        if (right) {
          const r = this.ring[first + 1] + (this.ring[second + 1] - this.ring[first + 1]) * fraction;
          right[index] = r;
        }
      }

      cursor += this.rate;
    }

    this.readPosition = cursor;
    this.reportedFrames += frames;
    if (this.reportedFrames >= REPORT_INTERVAL_FRAMES) {
      this.reportedFrames = 0;
      this.port.postMessage({
        readPosition: cursor,
        frames,
        underrunFrames: this.starvedFrames,
      });
      this.starvedFrames = 0;
    }
    return true;
  }
}

registerProcessor('osu-hitsound', HitsoundProcessor);
