<script setup>
// 加载页：输入 BID、可选转谱模式，然后交给 preview.js 加载会话。
// 加载期间显示后端透出的下载进度：OSZ 动辄几十 MiB，没有进度条会以为卡死了。
import { CONVERT_MODES, loadPreview, progressLabel, state } from '../preview.js';
</script>

<template>
  <section class="min-h-0 flex-1 overflow-auto">
    <div class="mx-auto w-full max-w-[720px] px-5 pt-[9vh] pb-10 sm:px-7">
      <div class="mb-9 flex items-center gap-3.5">
        <span class="grid h-12 w-12 shrink-0 place-items-center rounded-xl bg-[#ff5f45] text-[22px] font-extrabold text-white">
          o!
        </span>
        <div>
          <small class="text-[10px] tracking-[.16em] text-[#ff5f45]">LOCAL RENDER ENGINE</small>
          <h1 class="m-0 text-[26px] leading-tight font-bold text-white">谱面预览</h1>
        </div>
      </div>

      <form class="grid gap-4" @submit.prevent="loadPreview()">
        <label class="grid gap-2 text-[13px] font-bold text-neutral-300">
          谱面 BID
          <input
            v-model="state.bid"
            required
            inputmode="numeric"
            autocomplete="off"
            placeholder="例如 738063"
            class="h-11 rounded-md border border-neutral-700 bg-neutral-900 px-3 text-base font-normal text-white outline-none focus:border-[#ff5f45]"
          >
        </label>

        <label class="grid gap-2 text-[13px] font-bold text-neutral-300">
          转谱
          <select
            v-model="state.convert"
            class="h-11 rounded-md border border-neutral-700 bg-neutral-900 px-3 text-base font-normal text-white outline-none focus:border-[#ff5f45]"
          >
            <option value="">原模式</option>
            <option v-for="mode in CONVERT_MODES" :key="mode" :value="mode">{{ mode }}</option>
          </select>
        </label>

        <button
          type="submit"
          :disabled="state.loading"
          class="h-11 rounded-md bg-[#ff5f45] font-bold text-white transition hover:brightness-105 disabled:opacity-60"
        >
          {{ state.loading ? '加载中...' : '加载预览' }}
        </button>
      </form>

      <div v-if="state.loading" class="mt-5" aria-live="polite">
        <div class="flex items-baseline justify-between text-xs text-neutral-400">
          <span>{{ progressLabel }}</span>
          <span v-if="state.progress.detail" class="font-mono text-[11px] text-neutral-500">
            {{ state.progress.detail }}
          </span>
        </div>
        <div class="mt-2 h-1.5 overflow-hidden rounded-full bg-neutral-800">
          <div
            class="h-full rounded-full bg-[#ff5f45] transition-[width] duration-200 ease-out"
            :class="state.progress.percent === null ? 'w-1/3 animate-pulse' : ''"
            :style="state.progress.percent === null ? null : { width: `${state.progress.percent}%` }"
          />
        </div>
      </div>

      <p
        v-if="!state.gpuAvailable"
        class="mt-5 rounded-md border border-amber-700/60 bg-amber-950/40 p-3 text-xs leading-relaxed text-amber-300"
      >
        当前浏览器没有可用的 WebGPU（<code>navigator.gpu</code> 为空），页面无法渲染。
        局域网访问必须走 HTTPS，可以打开
        <a class="underline" href="/gpu-check.html" target="_blank" rel="noreferrer">/gpu-check.html</a>
        查看具体原因。
      </p>

      <pre
        v-if="state.loadLogs.length"
        class="mt-5 max-h-32 overflow-auto font-mono text-[12px] leading-relaxed whitespace-pre-wrap text-[#ff9d8a]"
      >{{ state.loadLogs.join('\n') }}</pre>
    </div>
  </section>
</template>
