import assert from 'node:assert/strict';
import { test } from 'node:test';
import { checkWebGpuAvailability, readRenderBackend, saveRenderBackend, withRenderBackend } from '../src/webgpu.js';

test('WebGPU 预检区分安全上下文、API、适配器与设备失败，并释放探测设备', async (t) => {
  const cases = [
    { name: '非安全上下文', secure: false, expected: /安全上下文/, requests: 0 },
    { name: 'API 缺失', noApi: true, expected: /未提供 WebGPU/, requests: 0 },
    { name: 'API 存在但无适配器', hardware: null, fallback: null, expected: /没有可用的适配器/, requests: 2 },
    { name: '硬件可用', hardware: true, expected: '', requests: 1, destroyed: true },
    { name: '软件回退', hardware: null, fallback: true, expected: '', requests: 2, destroyed: true },
    { name: '硬件异常仍尝试回退', hardwareError: true, fallback: true, expected: '', requests: 2, destroyed: true },
    { name: '画布失败', hardware: true, noCanvas: true, expected: /画布/, requests: 1 },
    { name: '设备失败', hardware: true, deviceError: true, expected: /device unavailable/, requests: 1 },
    { name: '回退异常', hardware: null, fallbackError: true, expected: /adapter unavailable/, requests: 2 },
  ];
  for (const entry of cases) {
    await t.test(entry.name, async (t) => {
      let requests = 0;
      let destroyed = false;
      const adapter = {
        async requestDevice() {
          if (entry.deviceError) throw new Error('device unavailable');
          return { destroy() { destroyed = true; } };
        },
      };
      const replaceGlobal = (name, value) => {
        const descriptor = Object.getOwnPropertyDescriptor(globalThis, name);
        Object.defineProperty(globalThis, name, { configurable: true, value });
        t.after(() => {
          if (descriptor) Object.defineProperty(globalThis, name, descriptor);
          else delete globalThis[name];
        });
      };
      replaceGlobal('isSecureContext', entry.secure !== false);
      replaceGlobal('navigator', { gpu: entry.noApi ? undefined : {
        async requestAdapter(options) {
          requests++;
          assert.equal(options.powerPreference, 'high-performance');
          if (options.forceFallbackAdapter ? entry.fallbackError : entry.hardwareError) {
            throw new Error('adapter unavailable');
          }
          return (options.forceFallbackAdapter ? entry.fallback : entry.hardware) ? adapter : null;
        },
      } });
      replaceGlobal('document', { createElement: () => ({ getContext: () => !entry.noCanvas }) });
      const result = await checkWebGpuAvailability();
      if (entry.expected instanceof RegExp) assert.match(result, entry.expected);
      else assert.equal(result, entry.expected);
      assert.equal(requests, entry.requests);
      assert.equal(destroyed, Boolean(entry.destroyed));
    });
  }
});

test('后端选择优先 WebGPU，能力或实际创建失败时回退，选择 WebGL 2 时直接使用', async (t) => {
  const cases = [
    { name: '自动优先 WebGPU', preference: 'webgpu', expected: 'webgpu' },
    { name: '没有 WebGPU API', preference: 'webgpu', noGpu: true, expected: 'webgl2' },
    { name: 'HTTP 仍可使用 WebGL 2', preference: 'webgpu', insecure: true, expected: 'webgl2' },
    { name: '实际 WebGPU 管线失败', preference: 'webgpu', createError: true, expected: 'webgl2' },
    { name: '强制 WebGL 2', preference: 'webgl2', expected: 'webgl2' },
    { name: '强制 WebGL 2 失败', preference: 'webgl2', noGl: true, error: /WebGL 2/ },
    { name: '两种后端都不可用', preference: 'webgpu', noGpu: true, noGl: true, error: /WebGPU.*WebGL 2/ },
    { name: '取消创建不再回退', preference: 'webgpu', aborted: true, error: /cancelled/ },
  ];
  for (const entry of cases) {
    await t.test(entry.name, async (t) => {
      let lost = 0;
      let gpuRequests = 0;
      const created = [];
      const fallbacks = [];
      const globals = {
        isSecureContext: !entry.insecure,
        navigator: { gpu: entry.noGpu ? undefined : {
          async requestAdapter() {
            gpuRequests++;
            return { async requestDevice() { return { destroy() {} }; } };
          },
        } },
        document: { createElement: () => ({ getContext: (type) => type === 'webgpu' ? {} : entry.noGl ? null : {
          isContextLost: () => false,
          getExtension: () => ({ loseContext() { lost++; } }),
        } }) },
      };
      for (const [name, value] of Object.entries(globals)) {
        const original = Object.getOwnPropertyDescriptor(globalThis, name);
        Object.defineProperty(globalThis, name, { configurable: true, value });
        t.after(() => {
          if (original) Object.defineProperty(globalThis, name, original);
          else delete globalThis[name];
        });
      }
      const result = withRenderBackend(entry.preference, async (backend) => {
        created.push(backend);
        if (entry.aborted) throw new DOMException('cancelled', 'AbortError');
        if (entry.createError && backend === 'webgpu') throw new Error('pipeline failed');
        return backend;
      }, (error) => fallbacks.push(error));
      if (entry.error) await assert.rejects(result, entry.error);
      else assert.equal(await result, entry.expected);
      if (entry.preference === 'webgl2') assert.equal(gpuRequests, 0);
      if (entry.expected === 'webgpu' || entry.aborted) assert.deepEqual(created, ['webgpu']);
      if (entry.expected === 'webgl2') assert.equal(lost, 1);
      assert.equal(fallbacks.length, entry.preference === 'webgpu' && !entry.aborted && entry.expected !== 'webgpu' ? 1 : 0);
    });
  }
});

test('后端偏好保存与读取校验，存储不可用时默认 WebGPU', (t) => {
  const original = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');
  t.after(() => {
    if (original) Object.defineProperty(globalThis, 'localStorage', original);
    else delete globalThis.localStorage;
  });
  let stored = null;
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: {
    getItem: () => stored,
    setItem: (key, value) => { assert.equal(key, 'preview.renderBackend'); stored = value; },
  } });
  assert.equal(readRenderBackend(), 'webgpu');
  saveRenderBackend('webgl2');
  assert.equal(readRenderBackend(), 'webgl2');
  stored = 'unsupported';
  assert.equal(readRenderBackend(), 'webgpu');
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, get() { throw new Error('denied'); } });
  assert.equal(readRenderBackend(), 'webgpu');
  assert.doesNotThrow(() => saveRenderBackend('webgpu'));
});
