<script setup>
// 视频区域：WASM 直接画在这块 canvas 上。
//
// 点击画面即播放/暂停；只有进度条常驻，其余控件都在 ControlSheet 里。
import { onMounted, ref } from 'vue';
import { attachStage, state, togglePlay } from '../preview.js';

const viewport = ref(null);
const canvas = ref(null);

onMounted(() => attachStage({ viewport: viewport.value, canvas: canvas.value }));
</script>

<template>
  <div
    ref="viewport"
    class="relative grid min-h-0 flex-1 cursor-pointer place-items-center overflow-hidden bg-black p-1 sm:p-3"
    @click="togglePlay()"
  >
    <canvas ref="canvas" class="block bg-[#12151b]" />
    <div
      v-if="state.fastForwarding || state.rewinding"
      class="pointer-events-none absolute top-3 left-1/2 -translate-x-1/2 rounded-full bg-black/70 px-3 py-1 text-[11px] text-white"
    >
      {{ state.fastForwarding ? '长按右键：3 倍速' : '长按左键：倒带' }}
    </div>
    <div
      v-if="!state.rendered || state.renderError"
      class="pointer-events-none absolute inset-0 grid place-items-center px-4 text-center text-sm text-neutral-500"
    >
      {{ state.renderError || '等待渲染' }}
    </div>
  </div>
</template>
