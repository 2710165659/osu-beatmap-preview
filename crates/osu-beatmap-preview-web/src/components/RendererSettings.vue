<script setup>
// 加载页和播放设置共用后端选择，可直接指定 WebGL 2 进行调试。
import ChipGroup from './ChipGroup.vue';
import { state, setRenderBackend } from '../preview.js';
import { RENDER_BACKENDS } from '../webgpu.js';
</script>

<template>
  <fieldset :disabled="state.loading || state.rendererSwitching" class="m-0 grid min-w-0 gap-2 border-0 p-0 disabled:opacity-60">
    <legend class="sr-only">渲染后端</legend>
    <div class="grid grid-cols-[52px_1fr] items-center gap-2">
      <span class="text-xs text-neutral-400">渲染后端</span>
      <ChipGroup :model-value="state.renderBackend" :options="RENDER_BACKENDS" @update:model-value="setRenderBackend" />
    </div>
    <p v-if="state.rendererSwitching" class="m-0 pl-[60px] text-[11px] text-neutral-500" role="status">正在切换…</p>
    <p v-if="state.rendererError" role="alert" class="m-0 pl-[60px] text-[11px] text-amber-300">{{ state.rendererError }}</p>
  </fieldset>
</template>
