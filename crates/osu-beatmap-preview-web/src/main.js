// Vue 前端源码目录：`npm run build` 把这里的单文件组件编译到 dist/，
// 由 backend/server.js 作为静态站点托管。
import { createApp } from 'vue';
import App from './App.vue';
import './style.css';

createApp(App).mount('#app');
