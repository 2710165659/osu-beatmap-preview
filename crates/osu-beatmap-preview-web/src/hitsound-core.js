// 纯逻辑部分：采样格式转换与环形缓冲区的索引换算。
//
// 拆出来是为了能在 Node 里直接单测——这些换算是「声音和画面对不齐」最容易出错的地方，
// 而浏览器里的 AudioWorklet 没法在 Node 里跑。

/**
 * 环形缓冲区的配置。
 *
 * 长度取 2 的整数次幂，这样内核可以用位运算取模（`index & mask`），
 * 在音频线程里比 `%` 便宜得多；2 秒的容量足够覆盖主线程一整帧的抖动。
 */
export function createRingConfig(sampleRate, seconds = 2) {
  const rate = Number.isFinite(sampleRate) && sampleRate > 0 ? Math.floor(sampleRate) : 48000;
  let length = 1;
  const wanted = Math.max(1024, rate * seconds);
  while (length < wanted) length *= 2;
  return { sampleRate: rate, length, mask: length - 1 };
}

/** 单声道 PCM 交错化成双声道：内核统一按立体声读取。 */
export function interleaveMono(channel) {
  const output = new Float32Array(channel.length * 2);
  for (let index = 0; index < channel.length; index++) {
    const value = channel[index];
    output[index * 2] = value;
    output[index * 2 + 1] = value;
  }
  return output;
}

/** 立体声 PCM 交错化成 [L, R, L, R, ...]。 */
export function interleaveStereo(left, right) {
  const frames = Math.min(left.length, right.length);
  const output = new Float32Array(frames * 2);
  for (let index = 0; index < frames; index++) {
    output[index * 2] = left[index];
    output[index * 2 + 1] = right[index];
  }
  return output;
}

/** 把绝对写入帧号转成环形坐标（内核读取指针用的坐标系）。 */
export function wrapReadFrame(absoluteFrame, ringLength) {
  const length = ringLength > 0 ? ringLength : 1;
  return ((absoluteFrame % length) + length) % length;
}

/**
 * 算出「绝对目标帧」在已解析区间里的相对下标（负数表示落在区间之前）。
 *
 * 位置判断全部以绝对帧号为准：环形绕回只影响读写指针的表示，不影响这里的比较。
 */
export function resolvedOffset(frame, resolvedFrom) {
  return frame - resolvedFrom;
}

/** 采样帧号换算成毫秒。 */
export function framesToMs(frames, sampleRate) {
  const rate = sampleRate > 0 ? sampleRate : 48000;
  return (frames * 1000) / rate;
}

/** 毫秒换算成采样帧号。 */
export function msToFrames(milliseconds, sampleRate) {
  const rate = sampleRate > 0 ? sampleRate : 48000;
  return (milliseconds * rate) / 1000;
}

/** 把两个环形位置的距离换算成「还剩多少采样帧可读」（考虑绕回）。 */
export function bufferedFrames(readFrame, writeFrames, ringLength) {
  const length = ringLength > 0 ? ringLength : 1;
  // 用取模而不是 `%`：写入位置绕回后 `write - read` 会是负数，`%` 会保留负号。
  return ((writeFrames - readFrame) % length + length) % length;
}
