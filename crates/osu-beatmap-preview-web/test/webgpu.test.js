import assert from 'node:assert/strict';
import { test } from 'node:test';
import { checkWebGpuAvailability } from '../src/webgpu.js';

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
