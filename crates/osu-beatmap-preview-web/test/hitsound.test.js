// 音频能力降级及实际消费内核的回归测试；模拟设备调度，不替代浏览器播放验证。
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createAudioContext, createAudioOutput } from '../src/hitsound.js';

let Processor;
globalThis.AudioWorkletProcessor = class {
  constructor() { this.port = {}; }
};
globalThis.registerProcessor = (_name, implementation) => { Processor = implementation; };
await import('../src/hitsound-worklet.js');
delete globalThis.AudioWorkletProcessor;
delete globalThis.registerProcessor;

function environment(t, { shared = true, worklet = true, loadFails = false, nodeFails = false, cloneFails = false, scriptFails = false } = {}) {
  const keys = ['AudioContext', 'AudioWorkletNode', 'SharedArrayBuffer', 'webkitAudioContext'];
  const originals = keys.map((key) => Object.getOwnPropertyDescriptor(globalThis, key));
  t.after(() => keys.forEach((key, index) => {
    if (originals[index]) Object.defineProperty(globalThis, key, originals[index]);
    else delete globalThis[key];
  }));
  const define = (key, value) => Object.defineProperty(globalThis, key, { value, writable: true, configurable: true });
  if (!shared) define('SharedArrayBuffer', undefined);
  const nodes = [];
  class Node {
    connect() {}
    disconnect() { this.disconnected = true; }
  }
  define('AudioContext', class {
    constructor() {
      this.sampleRate = 48000;
      this.currentTime = 0;
      this.destination = {};
      if (worklet) this.audioWorklet = { addModule: async () => {
        if (loadFails) throw new Error('加载失败');
      } };
    }
    createScriptProcessor() {
      if (scriptFails) throw new Error('节点失败');
      const node = new Node();
      nodes.push(node);
      return node;
    }
  });
  define('AudioWorkletNode', worklet ? class extends Node {
    constructor() {
      super();
      if (nodeFails) throw new Error('节点失败');
      this.processor = new Processor();
      this.port = {
        postMessage: (data, transfer = []) => {
          if (cloneFails && data.control) throw new Error('共享内存不可传输');
          this.processor.port.onmessage({ data: structuredClone(data, { transfer }) });
        },
        close: () => { this.port.closed = true; },
      };
      this.processor.port.postMessage = (data) => this.port.onmessage?.({ data });
      nodes.push(this);
    }
  } : undefined);
  return nodes;
}

function sessionStub() {
  return {
    epoch: 1,
    clocks: [],
    samples: new Float32Array(),
    audioEpoch() { return this.epoch; },
    rate() { return 1; },
    pullAudio() {
      if (this.resetOnPull) {
        this.epoch++;
        this.resetOnPull = false;
      }
      const samples = this.samples;
      this.samples = new Float32Array();
      return samples;
    },
    onAudioClock(position) { this.clocks.push(position); },
  };
}

function render(output, context, frames = 512, playbackTime = context.currentTime) {
  const channels = [new Float32Array(frames), new Float32Array(frames)];
  if (output.mode === 'script') {
    output.node.onaudioprocess({ playbackTime, outputBuffer: { getChannelData: (index) => channels[index] } });
  } else {
    output.node.processor.process([], [channels]);
  }
  return channels;
}

test('按实际能力与初始化结果逐级选择输出，并释放失败节点', async (t) => {
  const cases = [
    [{}, 'shared'],
    [{ shared: false }, 'message'],
    [{ cloneFails: true }, 'message'],
    [{ worklet: false }, 'script'],
    [{ loadFails: true }, 'script'],
    [{ nodeFails: true }, 'script'],
    [{ worklet: false, scriptFails: true }, null],
  ];
  for (const [options, expected] of cases) {
    await t.test(JSON.stringify(options), async (t) => {
      const nodes = environment(t, options);
      const context = await createAudioContext('worklet.js');
      assert.ok(context);
      const output = createAudioOutput({ session: sessionStub(), context });
      assert.equal(output?.mode ?? null, expected);
      if (options.cloneFails) {
        assert.ok(nodes[0].disconnected);
        assert.ok(nodes[0].port.closed);
      }
      output?.close();
    });
  }
});

test('无音频上下文、构造失败与旧版 webkitAudioContext', async (t) => {
  environment(t);
  const Context = globalThis.AudioContext;
  globalThis.AudioContext = undefined;
  globalThis.webkitAudioContext = undefined;
  assert.equal(await createAudioContext('worklet.js'), null);
  globalThis.AudioContext = class { constructor() { throw new Error('不可用'); } };
  assert.equal(await createAudioContext('worklet.js'), null);
  globalThis.AudioContext = undefined;
  globalThis.webkitAudioContext = Context;
  assert.ok(await createAudioContext('worklet.js'));
});

test('三种输出保持统一 PCM、倍速、暂停与纪元；旧时钟不污染 seek', async (t) => {
  for (const options of [{}, { shared: false }, { worklet: false }]) {
    await t.test(JSON.stringify(options), async (t) => {
      environment(t, options);
      const session = sessionStub();
      const context = await createAudioContext('worklet.js');
      const output = createAudioOutput({ session, context });
      const source = Float32Array.from([1, -1, 3, -3, 5, -5]);
      session.samples = source;
      output.ensureBuffered();
      assert.equal(source.byteLength, 24);
      output.setRate(0.5);
      output.setRate(NaN);
      output.setPlaying(true);
      assert.deepEqual(render(output, context, 4).map((channel) => [...channel]), [[1, 2, 3, 4], [-1, -2, -3, -4]]);
      output.setPlaying(false);
      assert.deepEqual(render(output, context, 4).map((channel) => [...channel]), [[0, 0, 0, 0], [0, 0, 0, 0]]);

      output.setPlaying(true);
      session.epoch++;
      // seek 已更新会话，但主线程尚未补缓冲时也不能接受旧纪元回报。
      output.node.port?.onmessage({ data: { epoch: 1, readPosition: 999 } });
      session.samples = Float32Array.from([9, -9, 7, -7]);
      output.ensureBuffered();
      output.setRate(1);
      assert.deepEqual(render(output, context, 2).map((channel) => [...channel]), [[9, 7], [-9, -7]]);
      assert.deepEqual(session.clocks, []);

      session.resetOnPull = true;
      session.samples = Float32Array.from([6, -6]);
      output.ensureBuffered();
      assert.deepEqual(render(output, context, 2).map((channel) => [...channel]), [[6, 0], [-6, 0]]);
      if (output.mode === 'script') {
        context.currentTime = 1;
        output.ensureBuffered();
      } else render(output, context);
      assert.ok(session.clocks.length > 0);
      assert.ok(output.underrunFrames > 0);
      output.close();
      assert.ok(output.node.disconnected);
      if (output.node.port) assert.ok(output.node.port.closed);
      else assert.equal(output.node.onaudioprocess, null);
    });
  }
});

test('ScriptProcessor 只报告已经到达设备播放时刻的帧，seek 丢弃排队报告', async (t) => {
  environment(t, { worklet: false });
  const session = sessionStub();
  const context = await createAudioContext('worklet.js');
  const output = createAudioOutput({ session, context });
  session.samples = new Float32Array(4096);
  output.setPlaying(true);
  render(output, context, 1024, 0.1);
  context.currentTime = 0.11;
  output.ensureBuffered();
  assert.deepEqual(session.clocks, []);
  context.currentTime = 0.13;
  output.ensureBuffered();
  assert.deepEqual(session.clocks, [1024]);
  render(output, context, 1024, 0.2);
  session.epoch++;
  context.currentTime = 1;
  output.ensureBuffered();
  assert.deepEqual(session.clocks, [1024]);
  output.close();
});
