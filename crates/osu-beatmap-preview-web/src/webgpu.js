// 在下载谱面前验证浏览器的 WebGPU 适配器、设备与画布能力。
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
