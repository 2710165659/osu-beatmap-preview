// 音频线程的 PCM 消费端：优先读共享环形缓冲，也可通过消息接收普通 PCM 分块。
// 混音与时间轴由 WASM 决定；这里只按设备时钟消费并回报当前纪元的相对帧号。
import { readChunk, writeChunk } from './hitsound-core.js';

const REPORT_INTERVAL_FRAMES = 512;

class HitsoundProcessor extends AudioWorkletProcessor {
  constructor() {
    super();
    this.ring = null;
    this.control = null;
    this.mask = 0;
    this.writtenFrames = 0;
    this.readPosition = 0;
    this.epoch = 0;
    this.playing = false;
    this.rate = 1;
    this.reportedFrames = 0;
    this.starvedFrames = 0;
    this.port.onmessage = ({ data }) => {
      if (!data) return;
      if (data.ring) this.ring = data.ring;
      if (data.ringFrames) this.ring = new Float32Array(data.ringFrames * 2);
      if (data.control) this.control = data.control;
      if (typeof data.mask === 'number') this.mask = data.mask;
      if (data.command === 'readFrame') {
        this.readPosition = data.value;
        this.writtenFrames = 0;
        this.reportedFrames = 0;
        this.starvedFrames = 0;
      }
      if (data.epoch !== undefined) this.epoch = data.epoch;
      if (data.samples && this.ring) {
        this.writtenFrames += writeChunk(this.ring, this.mask, this.writtenFrames, data.samples);
      }
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
    const left = output[0];
    const right = output.length > 1 ? output[1] : null;
    if (!this.playing || !this.ring) {
      left.fill(0);
      if (right) right.fill(0);
      return true;
    }
    const writtenFrames = this.control ? Atomics.load(this.control, 0) : this.writtenFrames;
    const result = readChunk(this.ring, this.mask, this.readPosition, writtenFrames, this.rate, left, right);
    this.readPosition = result.readPosition;
    this.starvedFrames += result.underrunFrames;
    this.reportedFrames += left.length;
    if (this.reportedFrames >= REPORT_INTERVAL_FRAMES) {
      this.reportedFrames = 0;
      this.port.postMessage({
        epoch: this.epoch,
        readPosition: this.readPosition,
        underrunFrames: this.starvedFrames,
      });
      this.starvedFrames = 0;
    }
    return true;
  }
}

registerProcessor('osu-hitsound', HitsoundProcessor);
