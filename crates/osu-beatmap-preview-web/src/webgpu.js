// 浏览器渲染后端的能力探测与自动回退；每次实际创建使用独立 Canvas。
export const RENDER_BACKENDS = Object.freeze([
  { value: 'webgpu', label: 'WebGPU' },
  { value: 'webgl2', label: 'WebGL 2' },
]);

export const backendLabel = (backend) => RENDER_BACKENDS.find(({ value }) => value === backend)?.label ?? '';

export async function checkWebGpuAvailability() {
  if (!globalThis.isSecureContext) {
    return '当前页面不是安全上下文，WebGPU 需要 HTTPS 或 localhost。请改用 HTTPS 访问本站。';
  }
  if (!globalThis.navigator?.gpu) {
    return '当前浏览器未提供 WebGPU，请使用支持 WebGPU 的浏览器，并检查图形加速设置。';
  }
  try {
    // 与 WASM 会话保持相同的适配器优先级；高性能偏好也允许选择核显。
    const gpu = navigator.gpu;
    let adapter;
    try {
      adapter = await gpu.requestAdapter({ powerPreference: 'high-performance' });
    } catch {
      // 硬件请求失败时仍尝试浏览器提供的软件回退。
    }
    adapter ??= await gpu.requestAdapter({ powerPreference: 'high-performance', forceFallbackAdapter: true });
    if (!adapter) {
      return '浏览器提供了 WebGPU API，但没有可用的适配器。核显也支持 WebGPU，请检查浏览器图形加速、显卡驱动及兼容性屏蔽情况。';
    }
    if (!document.createElement('canvas').getContext('webgpu')) {
      return '浏览器无法创建 WebGPU 画布，当前环境无法渲染预览。';
    }
    // 提前发现设备创建失败；探测设备不参与渲染，立即释放。
    const device = await adapter.requestDevice();
    device.destroy();
    return '';
  } catch (error) {
    return `WebGPU 检测失败：${error?.message ?? error}`;
  }
}

export function checkWebGl2Availability() {
  try {
    const context = document.createElement('canvas').getContext('webgl2');
    if (!context || context.isContextLost()) return '当前浏览器无法创建 WebGL 2 画布，请检查图形加速设置。';
    // 探测上下文不参与绘制，及时释放，避免反复检测耗尽浏览器的上下文数量。
    context.getExtension('WEBGL_lose_context')?.loseContext();
    return '';
  } catch (error) {
    return `WebGL 2 检测失败：${error?.message ?? error}`;
  }
}

/** 分别保留两个后端的检测结果；空字符串表示可用，非空字符串是不可用原因。 */
export async function checkRenderBackendAvailability() {
  const [webgpu, webgl2] = await Promise.all([checkWebGpuAvailability(), checkWebGl2Availability()]);
  return { webgpu, webgl2 };
}

/** WebGPU 优先模式也捕获实际创建失败；手动选择 WebGL 2 时不探测 WebGPU。 */
export async function withRenderBackend(preference, create, onFallback = () => {}) {
  if (!RENDER_BACKENDS.some(({ value }) => value === preference)) throw new Error('无效的渲染后端');
  const candidates = preference === 'webgpu' ? ['webgpu', 'webgl2'] : ['webgl2'];
  const errors = [];
  for (const backend of candidates) {
    try {
      const error = backend === 'webgpu' ? await checkWebGpuAvailability() : checkWebGl2Availability();
      if (error) throw new Error(error);
      return await create(backend);
    } catch (error) {
      if (error?.name === 'AbortError') throw error;
      errors.push(`${backendLabel(backend)}：${error?.message ?? error}`);
      if (preference === 'webgpu' && backend === 'webgpu') onFallback(errors[0]);
    }
  }
  throw new Error(errors.join('；'));
}
