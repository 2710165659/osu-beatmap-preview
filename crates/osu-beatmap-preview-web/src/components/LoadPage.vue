<script setup>
// 加载页显式区分普通预览与回放，谱面文本和媒体包始终是独立的可选输入。
import { reactive, watch } from 'vue';
import ReplayInput from './ReplayInput.vue';
import {
  CONVERT_MODES, clearLocalFile, loadPreview, progressDetail, progressLabel,
  progressPercent, selectLocalFile, setPreviewType, state, replayRequested,
  checkWebGpu, GPU_CHECK_URL,
} from '../preview.js';

const errors = reactive({ osu: '', osz: '' });
const fileInputs = {};
const fileKinds = ['osu', 'osz'];
// 深链导航会清理本地资源，同步清空原生输入框，避免文件名与实际字节不一致。
for (const kind of fileKinds) {
  watch(() => state[kind === 'osu' ? 'localOsuName' : 'localOszName'], (name) => {
    if (!name && fileInputs[kind]) fileInputs[kind].value = '';
  });
}

async function onFileChange(kind, event) {
  errors[kind] = '';
  const file = event.target.files?.[0];
  if (!file) { clearLocalFile(kind); return; }
  try { await selectLocalFile(kind, file); }
  catch (error) {
    clearLocalFile(kind);
    errors[kind] = String(error?.message ?? error);
    event.target.value = '';
  }
}

function removeFile(kind) {
  clearLocalFile(kind);
  errors[kind] = '';
  if (fileInputs[kind]) fileInputs[kind].value = '';
}
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
          <h1 class="m-0 text-[26px] leading-tight font-bold text-white">{{ replayRequested ? '回放预览' : '谱面预览' }}</h1>
        </div>
      </div>

      <div class="mb-6 grid grid-cols-2 gap-1 rounded-lg border border-neutral-800 bg-neutral-950 p-1" role="group" aria-label="预览模式">
        <button v-for="item in [{ type: 'osu', label: '谱面预览' }, { type: 'osr', label: '回放预览' }]" :key="item.type"
          type="button" :aria-pressed="state.previewType === item.type" :disabled="state.loading"
          class="rounded-md px-3 py-2.5 text-sm font-bold transition-colors disabled:opacity-60"
          :class="state.previewType === item.type ? 'bg-[#ff5f45] text-white' : 'text-neutral-400 hover:bg-neutral-800 hover:text-white'"
          @click="setPreviewType(item.type)">{{ item.label }}</button>
      </div>

      <div v-if="state.gpuStatus !== 'ready'" role="status" aria-live="polite"
        class="mb-6 rounded-md border border-amber-700/60 bg-amber-950/40 p-3 text-xs leading-relaxed text-amber-300">
        <template v-if="state.gpuStatus === 'checking'">正在检查 WebGPU 适配器与设备…</template>
        <template v-else>
          <p class="m-0">{{ state.gpuError }}</p>
          <div class="mt-2 flex flex-wrap items-center gap-4">
            <a class="underline" :href="GPU_CHECK_URL" target="_blank" rel="noreferrer">打开 WebGPU 自检</a>
            <button type="button" class="underline" @click="checkWebGpu()">重新检测</button>
          </div>
        </template>
      </div>

      <form class="grid gap-4" @submit.prevent="loadPreview()">
        <ReplayInput v-if="replayRequested" />
        <label v-if="!replayRequested" class="grid gap-2 text-[13px] font-bold text-neutral-300">
          谱面 BID
          <input
            v-model="state.bid"
            inputmode="numeric"
            autocomplete="off"
            placeholder="例如 738063；选择 .osz 时可指定难度"
            class="h-11 rounded-md border border-neutral-700 bg-neutral-900 px-3 text-base font-normal text-white outline-none focus:border-[#ff5f45]"
          >
        </label>

        <label v-if="!replayRequested" class="grid gap-2 text-[13px] font-bold text-neutral-300">
          转谱
          <select
            v-model="state.convert"
            class="h-11 rounded-md border border-neutral-700 bg-neutral-900 px-3 text-base font-normal text-white outline-none focus:border-[#ff5f45]"
          >
            <option value="">原模式</option>
            <option v-for="mode in CONVERT_MODES" :key="mode" :value="mode">{{ mode }}</option>
          </select>
        </label>

        <div class="flex items-center gap-3 text-[11px] tracking-wider text-neutral-600 select-none">
          <span class="h-px flex-1 bg-neutral-800"></span>
          可选本地资源
          <span class="h-px flex-1 bg-neutral-800"></span>
        </div>

        <div class="grid min-w-0 gap-3 sm:grid-cols-2">
          <div v-for="kind in fileKinds" :key="kind" class="grid min-w-0 content-start gap-2">
            <label class="grid min-w-0 gap-2 text-[13px] font-bold text-neutral-300">
              {{ kind === 'osu' ? '谱面文件 .osu（可选）' : '媒体包 .osz（可选）' }}
              <input :ref="(element) => { fileInputs[kind] = element; }" type="file" :accept="`.${kind}`"
                :disabled="state.loading"
                class="block w-full min-w-0 rounded-md border border-neutral-700 bg-neutral-900 px-3 py-2.5 text-[13px] font-normal text-neutral-400 file:mr-2 file:h-7 file:rounded file:border-0 file:bg-neutral-800 file:px-2 file:text-[12px] file:font-bold file:text-white"
                @change="onFileChange(kind, $event)">
            </label>
            <p v-if="state[kind === 'osu' ? 'localOsuName' : 'localOszName']" class="m-0 flex min-w-0 items-start gap-2 text-xs text-neutral-400">
              <span class="min-w-0 flex-1 break-all">{{ state[kind === 'osu' ? 'localOsuName' : 'localOszName'] }}</span>
              <button type="button" :disabled="state.loading" class="shrink-0 font-bold text-[#ff5f45] hover:underline" @click="removeFile(kind)">移除 .{{ kind }}</button>
            </p>
            <p v-if="errors[kind]" role="alert" class="m-0 text-xs text-red-300">{{ errors[kind] }}</p>
          </div>
        </div>

        <label v-if="!replayRequested && state.localOszName && !state.localOsuName && state.localDifficulties.length" class="grid gap-2 text-[13px] font-bold text-neutral-300">
          难度（本地 .osz）
          <select
            v-model="state.localDifficulty"
            class="h-11 rounded-md border border-neutral-700 bg-neutral-900 px-3 text-base font-normal text-white outline-none focus:border-[#ff5f45]"
          >
            <option v-for="item in state.localDifficulties" :key="item.entry" :value="item.entry">
              {{ item.label }}
            </option>
          </select>
        </label>

        <button
          type="submit"
          :disabled="state.gpuStatus !== 'ready' || state.loading || state.replaySelecting || state.localOsuSelecting || state.localOszSelecting"
          class="h-11 rounded-md bg-[#ff5f45] font-bold text-white transition hover:brightness-105 disabled:opacity-60"
        >
          {{ state.loading ? '加载中...' : '加载预览' }}
        </button>
      </form>

      <div v-if="state.loading" class="mt-5" aria-live="polite">
        <div class="flex items-baseline justify-between gap-3 text-xs text-neutral-400">
          <span class="truncate">{{ progressLabel }}</span>
          <span v-if="progressDetail" class="shrink-0 font-mono text-[11px] text-neutral-500">
            {{ progressDetail }}
          </span>
        </div>
        <div class="mt-2 flex items-center gap-2">
          <div class="h-1.5 min-w-0 flex-1 overflow-hidden rounded-full bg-neutral-800">
            <div
              class="h-full rounded-full bg-[#ff5f45] transition-[width] duration-200 ease-out"
              :class="progressPercent === null ? 'w-1/3 animate-pulse' : ''"
              :style="progressPercent === null ? null : { width: `${progressPercent}%` }"
            />
          </div>
          <span v-if="progressPercent !== null" class="w-9 shrink-0 text-right font-mono text-[11px] text-neutral-400">
            {{ progressPercent }}%
          </span>
        </div>
      </div>

      <pre
        v-if="state.loadLogs.length"
        class="mt-5 max-h-32 overflow-auto font-mono text-[12px] leading-relaxed whitespace-pre-wrap text-[#ff9d8a]"
      >{{ state.loadLogs.join('\n') }}</pre>
      <a v-if="state.loadLogs.length" class="mt-2 inline-block text-xs text-amber-300 underline"
        :href="GPU_CHECK_URL" target="_blank" rel="noreferrer">打开 WebGPU 自检</a>
    </div>
  </section>
</template>
