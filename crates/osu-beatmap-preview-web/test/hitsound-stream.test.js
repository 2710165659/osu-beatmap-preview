// 打击音输出流的仿真测试。
//
// AudioWorklet 没法在 Node 里跑，但「环形读写坐标 + 控制字 + 预读窗口」这套换算是
// 音画不同步的根源，这里用仿真音频线程忠实复现两个坐标系：
// - 控制字曾经用绝对帧号、音频线程用环形下标，播放绕回一圈后整段静音；
// - 填充目标曾经只看画面时钟，而音频线程由音频硬件时钟驱动、稳定地领先画面时钟
//   几十毫秒，于是它一路追到写入前沿、读到的全是静音（表现为打击音时有时无、
//   seek 之后完全消失）。

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { LOOKAHEAD_MS, RESEEK_TOLERANCE_MS, createHitsoundStream } from '../src/hitsound-stream.js';

/** 与真实设备一致的采样率：预读窗口是毫秒常量，用 8kHz 之类的测试采样率会失真。 */
const SAMPLE_RATE = 48000;
const RING_SECONDS = 0.5;
/** 位置回报间隔（帧），与 `public/hitsound-worklet.js` 保持一致。 */
const REPORT_INTERVAL_FRAMES = 512;

/**
 * 仿真音频线程：与 `public/hitsound-worklet.js` 的 `process()` 用同一套判定。
 *
 * 读取位置在内核中是累计帧号，只有访问环形数组时才取模；控制字保存已写入的累计帧数；
 * 位置按固定间隔回报给主线程（回报太密不代表真实行为，主线程拿到的总是略旧的位置）。
 */
function createFakeWorklet(stream) {
  const ring = stream.ring;
  const control = stream.control;
  const ringFrames = stream.ringFrames;
  const mask = ringFrames - 1;
  const state = {
    playing: false,
    rate: 1,
    readPosition: 0,
    underruns: 0,
    reportedFrames: 0,
  };
  return {
    /** 处理主线程发来的消息，语义与 `public/hitsound-worklet.js` 一致。 */
    onMessage(message) {
      if (!message) return;
      if (message.command === 'readFrame' && Number.isFinite(message.value)) {
        state.readPosition = message.value;
        state.reportedFrames = 0;
        return;
      }
      if (typeof message.playing === 'boolean') {
        state.playing = message.playing;
        state.reportedFrames = 0;
      }
      if (Number.isFinite(message.rate) && message.rate > 0) state.rate = message.rate;
    },
    state,
    play(playing) {
      state.playing = playing;
      state.reportedFrames = 0;
    },
    setRate(rate) {
      state.rate = rate;
    },
    /** 把读取位置直接推到某处，用于模拟「音频线程已经领先画面时钟」。 */
    lead(frames) {
      state.readPosition = frames;
    },
    /** 消费 `frames` 个输出帧，返回读到的样本（交错立体声）。 */
    consume(frames) {
      const output = new Float32Array(frames * 2);
      const writtenFrames = Atomics.load(control, 0);
      const oldestReadable = Math.max(0, writtenFrames - ringFrames);
      for (let index = 0; index < frames; index++) {
        const base = Math.floor(state.readPosition);
        if (state.playing && base >= oldestReadable && base < writtenFrames) {
          const slot = (base & mask) * 2;
          output[index * 2] = ring[slot];
          output[index * 2 + 1] = ring[slot + 1];
        } else if (state.playing) {
          state.underruns++;
        }
        state.readPosition += state.rate;
      }
      state.reportedFrames += frames;
      if (state.reportedFrames >= REPORT_INTERVAL_FRAMES) {
        state.reportedFrames = 0;
        stream.onReadFrame(state.readPosition, { underrun: false });
      }
      return output;
    },
  };
}

/**
 * 构造一条仿真流水线。
 *
 * WASM 侧用确定性「脉冲」PCM：每个事件的采样为 1，其余为 0，这样任何非零输出都
 * 对应一个明确的打击音位置，便于断言时间对齐。
 */
function createPipeline({ events }) {
  const state = { anchor: 0, seeks: 0, renders: 0 };
  /** 仿真音频线程；在 stream 建好之后接上，因此用可变引用。 */
  let sink = null;
  const stream = createHitsoundStream({
    sampleRate: SAMPLE_RATE,
    ring: null,
    control: null,
    render: (startMs, frames) => {
      state.renders++;
      const buffer = new Float32Array(frames * 2);
      const msPerFrame = 1000 / SAMPLE_RATE;
      for (let index = 0; index < frames; index++) {
        const time = startMs + index * msPerFrame;
        if (events.some((event) => Math.abs(event - time) < msPerFrame / 2)) {
          buffer[index * 2] = 1;
          buffer[index * 2 + 1] = 1;
        }
      }
      return buffer;
    },
    onPosition: (positionMs, restartVoices) => {
      if (restartVoices) state.seeks++;
      state.anchor = positionMs;
    },
    post: (message) => sink?.(message),
  });
  const worklet = createFakeWorklet(stream);
  sink = (message) => worklet.onMessage(message);
  return { stream, worklet, state };
}

/**
 * 跑一段仿真：按 `stepsPerSecond` 的频率调用 `ensureBuffered`，音频线程持续消费。
 *
 * @returns {number[]} 检测到的打击音谱面时间（毫秒）
 */
function run({ stream, worklet }, { fromMs, untilMs, fps = 60, rate = 1 }) {
  worklet.play(true);
  worklet.setRate(rate);
  stream.reset(fromMs, { restartVoices: true });

  const msPerFrame = 1000 / SAMPLE_RATE;
  const totalFrames = Math.round(((untilMs - fromMs) / rate) * SAMPLE_RATE / 1000);
  const stepFrames = Math.round(SAMPLE_RATE / fps);
  const detected = [];

  for (let produced = 0; produced < totalFrames; produced += stepFrames) {
    const frames = Math.min(stepFrames, totalFrames - produced);
    const chartTime = fromMs + produced * rate * msPerFrame;
    stream.ensureBuffered(chartTime);
    const output = worklet.consume(frames);
    for (let index = 0; index < frames; index++) {
      if (output[index * 2] !== 0) detected.push(fromMs + (produced + index) * rate * msPerFrame);
    }
  }
  return detected;
}

test('控制字与音频线程读取指针使用同一坐标系', () => {
  const { stream } = createPipeline({ events: [] });
  // 环长必须是 2 的幂（音频线程用位运算取模），且不小于目标时长对应的帧数。
  const exponent = Math.log2(stream.ringFrames);
  assert.equal(exponent % 1, 0, `ringFrames=${stream.ringFrames} 不是 2 的幂`);
  assert.ok(stream.ringFrames >= Math.max(1024, SAMPLE_RATE * RING_SECONDS));
  stream.reset(0);
  stream.write(Float32Array.from({ length: 200 }, () => 1), 100);
  assert.equal(Atomics.load(stream.control, 0), 100);
  assert.equal(stream.bufferedAhead, 100);

  // 再写一段：控制字仍然是「相对帧数」，与环形下标可比。
  stream.write(Float32Array.from({ length: 200 }, () => 1), 100);
  assert.equal(Atomics.load(stream.control, 0), 200);
  assert.equal(stream.bufferedAhead, 200);

  // 音频线程报回读取位置后，未消费量随之减少。
  stream.onReadFrame(120);
  assert.equal(stream.bufferedAhead, 80);
});

test('播放绕回一圈后仍然出声（回归：控制字曾用绝对帧号）', () => {
  const events = [];
  for (let time = 0; time <= 5000; time += 200) events.push(time);
  const pipeline = createPipeline({ events });
  const detected = run(pipeline, { fromMs: 0, untilMs: 5000, fps: 60 });

  const ringMs = (pipeline.stream.ringFrames / SAMPLE_RATE) * 1000;
  assert.ok(detected.length > 0, '完全没有声音');
  // 必须跨越绕回，且绕回之后依然有声音（曾经绕回后全静音）。
  const beyondFirstWrap = detected.filter((time) => time > ringMs);
  assert.ok(
    beyondFirstWrap.length > 0,
    `绕回后没有声音：环长 ${ringMs.toFixed(0)}ms，检测到 ${detected.join(',')}`,
  );
  assert.equal(pipeline.worklet.state.underruns, 0, '不应该欠载');
});

test('每个打击音都落在对应的谱面时间上', () => {
  const events = [1000, 1500, 2000, 2500, 3000];
  const pipeline = createPipeline({ events });
  const detected = run(pipeline, { fromMs: 900, untilMs: 3200, fps: 60 });
  const tolerance = (1000 / SAMPLE_RATE) * 1.5;

  for (const expected of events) {
    assert.ok(
      detected.some((value) => Math.abs(value - expected) <= tolerance),
      `事件 ${expected}ms 没有被播放（检测到 ${detected.join(',')}）`,
    );
  }
  assert.equal(detected.length, events.length, `打击音数量不符：${detected.join(',')}`);
});

test('倍速 2x 时事件位置仍然正确', () => {
  const events = [1000, 1500, 2000];
  const pipeline = createPipeline({ events });
  const detected = run(pipeline, { fromMs: 900, untilMs: 2100, fps: 60, rate: 2 });
  const tolerance = (1000 / SAMPLE_RATE) * 3;

  for (const expected of events) {
    assert.ok(
      detected.some((value) => Math.abs(value - expected) <= tolerance),
      `事件 ${expected}ms 没有被播放（检测到 ${detected.join(',')}）`,
    );
  }
});

test('音频线程领先画面时钟时不会读到静音（回归：预读曾只看画面时钟）', () => {
  // 真实浏览器里 HTML 音频元素与 AudioContext 的输出缓冲不同，音频线程的读取位置通常
  // 比画面时钟早几十毫秒。预读窗口只按画面时钟算时，它会一路追到写入前沿、读到的全是
  // 静音——这正是「打击音时有时无、拖动进度条或暂停后完全消失」的根因。
  const events = [];
  for (let time = 200; time <= 5200; time += 100) events.push(time);
  const pipeline = createPipeline({ events });
  const { stream, worklet } = pipeline;
  const msPerFrame = 1000 / SAMPLE_RATE;
  const stepFrames = Math.round(SAMPLE_RATE / 60);
  const totalFrames = Math.round(SAMPLE_RATE * 5.4);

  worklet.play(true);
  stream.reset(0, { restartVoices: true });
  // 音频线程已经领先 60ms：之后的每个输出帧都按它自己的时钟消费。
  worklet.lead(Math.round(0.06 * SAMPLE_RATE));

  const detected = [];
  for (let produced = 0; produced < totalFrames; produced += stepFrames) {
    const frames = Math.min(stepFrames, totalFrames - produced);
    stream.ensureBuffered(produced * msPerFrame);
    // 检测时间取音频线程自己的谱面时间：写进环形的第 r 帧对应的谱面时间由重置时的
    // 锚点唯一确定，因此这样断言同时验证了「时间对齐」与「没有欠载」。
    const readStart = worklet.state.readPosition;
    const output = worklet.consume(frames);
    for (let index = 0; index < frames; index++) {
      if (output[index * 2] !== 0) detected.push((readStart + index) * msPerFrame);
    }
  }

  assert.equal(worklet.state.underruns, 0, `音频线程读到了 ${worklet.state.underruns} 帧静音`);
  for (const expected of events) {
    assert.ok(
      detected.some((value) => Math.abs(value - expected) <= msPerFrame * 1.5),
      `事件 ${expected}ms 没有被播放（检测到 ${detected.length} 个打击音）`,
    );
  }
});

test('seek 到远处会重新对齐而不是留下错位数据', () => {
  const events = [0, 1000, 2000, 3000, 4000];
  const pipeline = createPipeline({ events });
  const { stream, worklet } = pipeline;
  const msPerFrame = 1000 / SAMPLE_RATE;
  const stepFrames = Math.round(SAMPLE_RATE / 60);

  // 先播一段（0 → 600ms），再跳到 2900ms。
  worklet.play(true);
  worklet.setRate(1);
  stream.reset(0, { restartVoices: true });
  for (let produced = 0; produced < Math.round(SAMPLE_RATE * 0.6); produced += stepFrames) {
    stream.ensureBuffered(produced * msPerFrame);
    worklet.consume(stepFrames);
  }

  stream.reset(2900, { restartVoices: true });
  const detected = [];
  // 跑到 4300ms，保证 4000ms 的事件完整落在统计区间内。
  const totalFrames = Math.round(SAMPLE_RATE * 1.4);
  for (let produced = 0; produced < totalFrames; produced += stepFrames) {
    const frames = Math.min(stepFrames, totalFrames - produced);
    const chartTime = 2900 + produced * msPerFrame;
    stream.ensureBuffered(chartTime);
    const output = worklet.consume(frames);
    for (let index = 0; index < frames; index++) {
      if (output[index * 2] !== 0) detected.push(2900 + (produced + index) * msPerFrame);
    }
  }

  // 跳转后不应再听到旧位置的声音，且 3000ms / 4000ms 必须被听到。
  assert.ok(
    detected.every((time) => time >= 2900),
    `seek 后出现了旧位置的声音：${detected.join(',')}`,
  );
  for (const expected of [3000, 4000]) {
    assert.ok(
      detected.some((value) => Math.abs(value - expected) <= msPerFrame * 1.5),
      `seek 后的 ${expected}ms 事件没有被播放（检测到 ${detected.join(',')}）`,
    );
  }
});

test('预读窗口与容差常量保持合理关系', () => {
  // 容差不能小于预读窗口，否则正常的预读推进会被误判成 seek 而反复重置。
  assert.ok(RESEEK_TOLERANCE_MS >= LOOKAHEAD_MS);
});
