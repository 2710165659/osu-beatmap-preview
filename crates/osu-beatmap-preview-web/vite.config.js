// Vue 前端的构建配置。
//
// 只做三件事：编译 Vue 单文件组件、用 Tailwind 生成样式、把 public/ 原样拷进 dist/。
// 产物是纯静态文件，backend/server.js 直接托管 dist/，不需要任何运行时依赖。
import { defineConfig } from 'vite';
import vue from '@vitejs/plugin-vue';
import tailwindcss from '@tailwindcss/vite';

// `npm run dev` 时把后端接口代理到本地服务，前端单独跑也不用开跨域。
const backend = process.env.OSU_PREVIEW_BACKEND ?? 'http://127.0.0.1:8787';

export default defineConfig({
  plugins: [vue(), tailwindcss()],
  server: {
    proxy: {
      '/resource': { target: backend, changeOrigin: true },
    },
    // 打击音通过 SharedArrayBuffer 把 WASM 的混音结果共享给音频线程，浏览器只在
    // 跨源隔离下允许使用它。生产环境的响应头由 backend/server.js 提供，这里让
    // `npm run dev` 的开发服务器保持一致，否则开发时打击音会静音。
    headers: {
      'Cross-Origin-Opener-Policy': 'same-origin',
      'Cross-Origin-Embedder-Policy': 'require-corp',
    },
  },
  build: {
    // dist/ 是后端唯一托管的目录，构建前先清空，避免残留旧资源。
    outDir: 'dist',
    emptyOutDir: true,
  },
});
