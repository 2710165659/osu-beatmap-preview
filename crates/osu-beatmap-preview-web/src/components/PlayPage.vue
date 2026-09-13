<script setup>
// 播放页：顶部只保留返回、模式与状态；中间是视频，底部是常驻进度条。
// 播放按钮、画面参数与 Mod 全部收进 ControlSheet，默认不占位置。
import { computed } from 'vue';
import ControlSheet from './ControlSheet.vue';
import InfoBar from './InfoBar.vue';
import PlayerStage from './PlayerStage.vue';
import TimelineBar from './TimelineBar.vue';
import { backToLoad, progressDetail, progressLabel, progressPercent, setSheetOpen, showTimeline, state } from '../preview.js';

// 画面已经能看时音频/背景可能还在下：这时候不该再整页挡住，只在顶部留一条细进度。
const mediaLoading = computed(() => state.preparingMedia && state.progress.phase === 'media');
</script>

<template>
  <section
    class="flex min-h-0 flex-1 flex-col bg-[#0b0d10]"
    @mousemove="showTimeline()"
    @touchstart.passive="showTimeline()"
  >
    <header class="flex h-11 shrink-0 items-center gap-2 border-b border-neutral-800 bg-[#111419] px-2 sm:gap-3 sm:px-3">
      <button
        type="button"
        class="h-8 shrink-0 rounded-md border border-neutral-700 bg-neutral-900 px-2.5 text-xs text-neutral-200 transition hover:border-neutral-500"
        @click="backToLoad()"
      >
        返回加载
      </button>
      <strong class="shrink-0 text-[11px] tracking-[.14em] text-[#ff5f45]">{{ state.mode }}</strong>
      <span v-if="state.audioBlocked" class="shrink-0 rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] text-amber-300">
        静音播放中
      </span>
      <span class="ml-auto truncate text-[11px] text-neutral-400">{{ state.status }}</span>
      <button
        type="button"
        class="grid h-8 w-8 shrink-0 place-items-center rounded-md border text-base transition"
        :class="state.sheetOpen
          ? 'border-[#ff5f45] bg-[#ff5f45]/15 text-white'
          : 'border-neutral-700 bg-neutral-900 text-neutral-300 hover:border-neutral-500'"
        aria-label="播放设置"
        @click="setSheetOpen(!state.sheetOpen)"
      >
        ⚙
      </button>
    </header>

    <!-- 音频/背景还在后台下载：画面已经能看，所以只占一行，不挡视频。 -->
    <div v-if="mediaLoading" class="flex shrink-0 items-center gap-2 border-b border-neutral-800 bg-[#111419] px-2 py-1.5 sm:px-3">
      <span class="shrink-0 text-[11px] text-neutral-400">{{ progressLabel }}</span>
      <div class="h-1 min-w-0 flex-1 overflow-hidden rounded-full bg-neutral-800">
        <div
          class="h-full rounded-full bg-[#ff5f45] transition-[width] duration-200 ease-out"
          :class="progressPercent === null ? 'w-1/3 animate-pulse' : ''"
          :style="progressPercent === null ? null : { width: `${progressPercent}%` }"
        />
      </div>
      <span v-if="progressDetail" class="shrink-0 font-mono text-[10px] text-neutral-500">{{ progressDetail }}</span>
    </div>

    <InfoBar />
    <PlayerStage />
    <TimelineBar />
    <ControlSheet />
  </section>
</template>
