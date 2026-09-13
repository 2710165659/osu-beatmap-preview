// 打击音纯逻辑的单元测试：采样格式转换、环形索引换算与「是否已解析」判断。
//
// 这些换算是音画同步最容易出错的地方，而 AudioWorklet 本身无法在 Node 里运行，
// 所以把纯逻辑拆到 `hitsound-core.js` 并在 `npm test` 里覆盖。

import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  bufferedFrames,
  createRingConfig,
  framesToMs,
  interleaveStereo,
  msToFrames,
  samplePcmForWasm,
} from '../src/hitsound-core.js';

test('环形长度取 2 的整数次幂并覆盖目标时长', () => {
  const config = createRingConfig(48000, 2);
  assert.equal(config.sampleRate, 48000);
  assert.equal(Math.log2(config.length) % 1, 0, 'length 必须是 2 的幂');
  assert.ok(config.length >= 96000);
  assert.equal(config.mask, config.length - 1);
});

test('非法采样率回退到 48000', () => {
  assert.equal(createRingConfig(0).sampleRate, 48000);
  assert.equal(createRingConfig(NaN).sampleRate, 48000);
  assert.equal(createRingConfig(-1).sampleRate, 48000);
});

test('单声道样本按纯单声道交给内核', () => {
  // 回归：曾经把单声道先交错成双声道、却仍按 `channels = 1` 交进去，混音器会把数组
  // 当成长度翻倍的单声道样本（每个采样帧播两次），声音被拉长一倍、低一个八度——
  // Web 端 taiko（样本全是单声道）听上去「和原音不符、开二倍速才正常」的根因。
  const buffer = {
    numberOfChannels: 1,
    getChannelData: () => Float32Array.from([0.25, -0.5, 0.75]),
  };
  const { channels, samples } = samplePcmForWasm(buffer);
  assert.equal(channels, 1);
  assert.equal(samples.length, 3, 'mono 数组长度必须等于采样帧数');
  assert.deepEqual(Array.from(samples), [0.25, -0.5, 0.75]);
});

test('立体声样本交错成 L R 交给内核', () => {
  const buffer = {
    numberOfChannels: 2,
    getChannelData: (index) =>
      index === 0 ? Float32Array.from([1, 2]) : Float32Array.from([-1, -2]),
  };
  const { channels, samples } = samplePcmForWasm(buffer);
  assert.equal(channels, 2);
  assert.deepEqual(Array.from(samples), [1, -1, 2, -2]);
});

test('多声道样本只取前两个声道', () => {
  const buffer = {
    numberOfChannels: 6,
    getChannelData: (index) => Float32Array.from([index, index + 1]),
  };
  const { channels, samples } = samplePcmForWasm(buffer);
  assert.equal(channels, 2);
  assert.deepEqual(Array.from(samples), [0, 1, 1, 2]);
});

test('立体声交错按 L R 顺序排列', () => {
  const stereo = interleaveStereo(Float32Array.from([1, 2]), Float32Array.from([-1, -2]));
  assert.deepEqual(Array.from(stereo), [1, -1, 2, -2]);
});

test('立体声交错取两个声道较短的长度', () => {
  const stereo = interleaveStereo(Float32Array.from([1, 2, 3]), Float32Array.from([-1]));
  assert.deepEqual(Array.from(stereo), [1, -1]);
});

test('帧与毫秒互换与采样率一致', () => {
  assert.equal(msToFrames(1000, 48000), 48000);
  assert.equal(framesToMs(48000, 48000), 1000);
  assert.equal(msToFrames(500, 1000), 500);
  // 采样率为 0 时按默认采样率处理，除零不会产生 Infinity。
  assert.equal(framesToMs(48000, 0), 1000);
});

test('环形剩余量在绕回后仍然正确', () => {
  // 读 900、写 100（已绕回 1024 的环），剩余 224 帧。
  assert.equal(bufferedFrames(900, 100, 1024), 224);
  assert.equal(bufferedFrames(0, 0, 1024), 0);
  assert.equal(bufferedFrames(10, 20, 1024), 10);
});
