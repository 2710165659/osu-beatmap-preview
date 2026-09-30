// 纯逻辑部分：环形缓冲区的配置与写入换算。
//
// 拆出来是为了能在 Node 里直接单测——环形读写是「声音和画面对不齐」最容易出错的地方，
// 而浏览器里的 AudioWorklet 没法在 Node 里跑。混音与时间轴已经全部在 WASM 里，
// 这里只剩宿主侧的搬运数学。

/**
 * 环形缓冲区的配置。
 *
 * 长度取 2 的整数次幂，这样内核可以用位运算取模（`index & mask`），
 * 在音频线程里比 `%` 便宜得多；2 秒的容量足够覆盖主线程一整帧的抖动
 * （WASM 的预读窗口只有 170ms 左右）。
 */
export function createRingConfig(sampleRate, seconds = 2) {
  const rate = Number.isFinite(sampleRate) && sampleRate > 0 ? Math.floor(sampleRate) : 48000;
  let length = 1;
  const wanted = Math.max(1024, rate * seconds);
  while (length < wanted) length *= 2;
  return { sampleRate: rate, length, mask: length - 1 };
}

/**
 * 把一段交错立体声 PCM 写进环形缓冲。
 *
 * `startFrame` 是这段数据第 0 帧的相对帧号（自上次重置起的写入位置），下标用掩码
 * 取模；写入位置只增不减，绕回由音频线程用「已写入 - 环形容量」自行判断。
 *
 * @param {Float32Array} ring 环形缓冲（交错立体声，长度 = 帧数 × 2）
 * @param {number} mask 环形掩码（`createRingConfig` 的输出）
 * @param {number} startFrame 该段第 0 帧的相对帧号
 * @param {Float32Array} samples 交错立体声 PCM（长度 = 帧数 × 2）
 * @returns {number} 实际写入的采样帧数
 */
export function writeChunk(ring, mask, startFrame, samples) {
  const ringFrames = ring.length / 2;
  const frames = Math.min(Math.floor(samples.length / 2), ringFrames);
  for (let index = 0; index < frames; index += 1) {
    const slot = ((startFrame + index) & mask) * 2;
    ring[slot] = samples[index * 2];
    ring[slot + 1] = samples[index * 2 + 1];
  }
  return frames;
}
