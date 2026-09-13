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

/**
 * 把 AudioBuffer 转成 WASM 需要的 PCM 与声道数。
 *
 * WASM 侧的约定是：`channels == 1` 时数组是纯单声道（长度 = 采样帧数），`channels == 2`
 * 时是交错立体声（长度 = 采样帧数 × 2）。曾经把单声道先交错成双声道、却仍按
 * `channels = 1` 交进去，混音器会把它当成「长度翻倍的单声道样本」，鼓声被拉长一倍、
 * 低一个八度——这就是 Web 端 taiko（单声道样本）听上去「和原音不符、开二倍速才正常」
 * 的根因。
 */
export function samplePcmForWasm(buffer) {
  const channels = Math.min(2, buffer.numberOfChannels);
  if (channels <= 1) {
    return { channels: 1, samples: buffer.getChannelData(0) };
  }
  const samples = interleaveStereo(buffer.getChannelData(0), buffer.getChannelData(1));
  return { channels: 2, samples };
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
