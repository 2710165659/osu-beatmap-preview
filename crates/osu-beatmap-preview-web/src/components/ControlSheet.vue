<script setup>
// 播放设置抽屉：默认收起，视频区域因此保持最大。
//
// 手机上从底部弹出，桌面（lg）变成右下角浮层；四组内容分别是播放控制、
// 画面参数、Mod 与运行日志。
import { computed } from 'vue';
import ChipGroup from './ChipGroup.vue';
import {
  activeDaToken,
  commitDa,
  daVisible,
  FPS_CHOICES,
  modTokens,
  RESOLUTIONS,
  setDaValue,
  setFps,
  setResolution,
  setSheetOpen,
  setSpeed,
  SPEED_CHOICES,
  state,
  toggleMod,
} from '../preview.js';

const fpsOptions = FPS_CHOICES.map((value) => ({ value, label: `${value} FPS` }));
const speedOptions = SPEED_CHOICES.map((value) => ({ value, label: `${value}x` }));
const resolutionOptions = Object.entries(RESOLUTIONS).map(([key]) => ({ value: key, label: `${key}P` }));
const hasLogs = computed(() => state.playLogs.length > 0);

const chipClass = (active) => (active
  ? 'border-[#ff5f45] bg-[#ff5f45]/15 text-white'
  : 'border-neutral-700 bg-neutral-900 text-neutral-300 hover:border-neutral-500');
</script>

<template>
  <div v-if="state.sheetOpen" class="fixed inset-0 z-40">
    <div class="absolute inset-0 bg-black/60" @click="setSheetOpen(false)" />

    <div
      class="absolute inset-x-0 bottom-0 max-h-[80dvh] overflow-y-auto rounded-t-2xl border-t border-neutral-800 bg-[#111419] px-4 pt-3 pb-6 shadow-2xl
             lg:inset-x-auto lg:right-4 lg:bottom-4 lg:max-h-[72dvh] lg:w-[380px] lg:rounded-2xl lg:border"
    >
      <div class="mx-auto mb-4 h-1 w-10 rounded-full bg-neutral-700 lg:hidden" />

      <section class="mb-5 grid gap-3">
        <h2 class="text-[11px] tracking-[.14em] text-neutral-500 uppercase">画面</h2>
        <div class="grid grid-cols-[52px_1fr] items-center gap-2">
          <span class="text-xs text-neutral-400">帧率</span>
          <ChipGroup :model-value="state.fps" :options="fpsOptions" @update:model-value="setFps" />
        </div>
        <div class="grid grid-cols-[52px_1fr] items-center gap-2">
          <span class="text-xs text-neutral-400">清晰度</span>
          <ChipGroup :model-value="state.resolution" :options="resolutionOptions" @update:model-value="setResolution" />
        </div>
        <div class="grid grid-cols-[52px_1fr] items-center gap-2">
          <span class="text-xs text-neutral-400">倍速</span>
          <ChipGroup :model-value="state.speed" :options="speedOptions" @update:model-value="setSpeed" />
        </div>
      </section>

      <section class="mb-5">
        <h2 class="mb-2 text-[11px] tracking-[.14em] text-neutral-500 uppercase">Mod</h2>
        <div class="flex flex-wrap gap-1.5">
          <button
            v-for="token in modTokens"
            :key="token"
            type="button"
            class="rounded-md border px-2.5 py-1 text-xs transition"
            :class="chipClass(state.mods.includes(token))"
            :title="token === 'DA' ? 'Difficulty Adjust（可调 AR / CS）' : undefined"
            @click="toggleMod(token)"
          >
            {{ token }}
          </button>
        </div>

        <div v-if="daVisible" class="mt-3 grid gap-3 rounded-lg border border-dashed border-neutral-700 bg-[#0f1319] p-3">
          <p class="m-0 text-[11px] text-neutral-500">DA 参数（{{ activeDaToken() }}）</p>
          <label class="grid grid-cols-[24px_minmax(0,1fr)_44px] items-center gap-2 text-xs text-neutral-300">
            AR
            <input
              type="range" min="-10" max="11" step="0.1" class="h-8 w-full cursor-pointer"
              :value="state.daAr"
              @input="setDaValue('ar', $event.target.value)"
              @change="commitDa()"
            >
            <output class="text-right font-mono">{{ state.daAr.toFixed(1) }}</output>
          </label>
          <label class="grid grid-cols-[24px_minmax(0,1fr)_44px] items-center gap-2 text-xs text-neutral-300">
            CS
            <input
              type="range" min="0" max="11" step="0.1" class="h-8 w-full cursor-pointer"
              :value="state.daCs"
              @input="setDaValue('cs', $event.target.value)"
              @change="commitDa()"
            >
            <output class="text-right font-mono">{{ state.daCs.toFixed(1) }}</output>
          </label>
        </div>
      </section>

      <section>
        <h2 class="mb-2 text-[11px] tracking-[.14em] text-neutral-500 uppercase">
          运行日志 <span v-if="!hasLogs" class="tracking-normal normal-case">（暂无）</span>
        </h2>
        <pre
          v-if="hasLogs"
          class="m-0 max-h-52 overflow-auto rounded-lg bg-[#0f1319] p-3 font-mono text-[11px] leading-relaxed whitespace-pre-wrap text-[#ff9d8a]"
        >{{ state.playLogs.join('\n') }}</pre>
      </section>
    </div>
  </div>
</template>
