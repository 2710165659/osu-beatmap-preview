// 环形缓冲纯逻辑的单元测试：配置换算与写入换算。
//
// 这些换算是音画同步最容易出错的地方，而 AudioWorklet 本身无法在 Node 里运行，
// 所以把纯逻辑拆到 `hitsound-core.js` 并在 `npm test` 里覆盖。混音、时间轴与
// 时钟已经全部在 WASM 里（Rust 侧有单测），宿主侧只剩搬运数学。

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { createRingConfig, writeChunk } from '../src/hitsound-core.js';

test('环形配置：长度为 2 的幂、掩码配套、坏采样率回退 48kHz', () => {
  const config = createRingConfig(48000, 2);
  assert.equal(config.sampleRate, 48000);
  assert.equal(config.length, 131072); // 48000×2 = 96000 → 向上取 2 的幂
  assert.equal(config.mask, config.length - 1);
  assert.equal(config.length & config.mask, 0);

  assert.equal(createRingConfig(0).sampleRate, 48000);
  assert.equal(createRingConfig(NaN).sampleRate, 48000);
  assert.equal(createRingConfig(-1).sampleRate, 48000);
  // 极低采样率也有保底长度，掩码才能用位运算取模。
  assert.ok(createRingConfig(8000, 0.001).length >= 1024);
});

test('写入：顺序写进相对帧号位置，交错立体声按帧搬两采样', () => {
  const config = createRingConfig(1000, 0.002); // 1024 帧
  const ring = new Float32Array(config.length * 2);
  const samples = Float32Array.from([1, 2, 3, 4, 5, 6]); // 3 帧
  const written = writeChunk(ring, config.mask, 0, samples);
  assert.equal(written, 3);
  assert.deepEqual([...ring.slice(0, 6)], [1, 2, 3, 4, 5, 6]);
});

test('写入：绕回时用掩码取模，帧数被环形容量截断', () => {
  const config = createRingConfig(1000, 0.002); // 1024 帧
  const ring = new Float32Array(config.length * 2);
  const tail = Float32Array.from([9, 9]);
  // 起点越过一圈整数倍后仍落在环内。
  assert.equal(writeChunk(ring, config.mask, config.length + 1, tail), 1);
  assert.equal(ring[2], 9);
  assert.equal(ring[3], 9);

  // 数据比环还大：只写得下一圈，且不越界。
  const huge = new Float32Array((config.length + 10) * 2);
  assert.equal(writeChunk(ring, config.mask, 0, huge), config.length);
});
