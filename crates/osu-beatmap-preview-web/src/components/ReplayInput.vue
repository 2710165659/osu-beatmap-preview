<script setup>
// 三种回放来源显式互斥；切换时清理本地字节和未完成解析，谱面资源可继续复用。
import { ref } from 'vue';
import { state, selectReplayFile, clearReplayFile } from '../preview.js';
const input = ref(null);
const error = ref('');
async function select(event) {
  error.value = '';
  const file = event.target.files?.[0];
  if (!file) { clearReplayFile(); return; }
  try { await selectReplayFile(file); }
  catch (reason) { clearReplayFile(); error.value = String(reason?.message ?? reason); event.target.value = ''; }
}
function remove() { clearReplayFile(); error.value = ''; if (input.value) input.value.value = ''; }
</script>

<template>
  <fieldset :disabled="state.loading" class="m-0 grid min-w-0 gap-3 border-0 p-0">
    <label class="grid gap-2 text-[13px] font-bold text-neutral-300">
      回放来源
      <select v-model="state.replayMethod" aria-label="回放来源" class="h-11 rounded-md border border-neutral-700 bg-neutral-900 px-3 text-base font-normal text-white outline-none focus:border-[#ff5f45]" @change="remove">
        <option value="file">本地 OSR（可选，不选则普通谱面预览）</option>
        <option value="score">成绩 ID / 官方成绩链接</option>
        <option value="player">玩家 + 谱面 ID</option>
      </select>
    </label>
    <label v-if="state.replayMethod === 'file'" class="grid gap-2 text-[13px] font-bold text-neutral-300">
      回放文件（可选 .osr）
      <input ref="input" type="file" accept=".osr" :disabled="state.loading" class="block w-full rounded-md border border-neutral-700 bg-neutral-900 px-3 py-2.5 text-[13px] font-normal text-neutral-400 file:mr-3 file:h-7 file:rounded file:border-0 file:bg-neutral-800 file:px-2.5 file:text-[12px] file:font-bold file:text-white" @change="select">
    </label>
    <label v-else-if="state.replayMethod === 'score'" class="grid gap-2 text-[13px] font-bold text-neutral-300">
      成绩 ID 或链接
      <input v-model="state.replayScoreId" type="text" autocomplete="off" placeholder="6086051920 或 https://osu.ppy.sh/scores/6086051920" class="h-11 min-w-0 rounded-md border border-neutral-700 bg-neutral-900 px-3 text-base font-normal text-white outline-none focus:border-[#ff5f45]">
    </label>
    <template v-else-if="state.replayMethod === 'player'">
      <div class="grid gap-3 sm:grid-cols-[130px_1fr]">
        <label class="grid gap-2 text-[13px] font-bold text-neutral-300">
          玩家标识
          <select v-model="state.replayUserKind" aria-label="玩家标识" class="h-11 rounded-md border border-neutral-700 bg-neutral-900 px-3 text-base font-normal text-white outline-none focus:border-[#ff5f45]">
            <option value="name">玩家名</option><option value="id">玩家 ID</option>
          </select>
        </label>
        <label class="grid gap-2 text-[13px] font-bold text-neutral-300">
          {{ state.replayUserKind === 'name' ? '玩家名' : '玩家 ID' }}
          <input v-model="state.replayUserValue" autocomplete="off" :placeholder="state.replayUserKind === 'name' ? '输入玩家名，数字名称也按名称查询' : '输入数字玩家 ID'" class="h-11 min-w-0 rounded-md border border-neutral-700 bg-neutral-900 px-3 text-base font-normal text-white outline-none focus:border-[#ff5f45]">
        </label>
      </div>
      <div class="grid gap-3 sm:grid-cols-2">
        <label class="grid gap-2 text-[13px] font-bold text-neutral-300">
          回放谱面 ID
          <input v-model="state.replayBid" inputmode="numeric" placeholder="例如 4903207（单张难度 ID）" class="h-11 min-w-0 rounded-md border border-neutral-700 bg-neutral-900 px-3 text-base font-normal text-white outline-none focus:border-[#ff5f45]">
        </label>
        <label class="grid gap-2 text-[13px] font-bold text-neutral-300">
          成绩选择
          <select v-model="state.replaySelection" aria-label="成绩选择" class="h-11 rounded-md border border-neutral-700 bg-neutral-900 px-3 text-base font-normal text-white outline-none focus:border-[#ff5f45]">
            <option value="best">最高分（默认）</option><option value="recent">最近一次（仅近期记录）</option>
          </select>
        </label>
      </div>
    </template>
    <p v-if="state.replayMethod !== 'file'" class="m-0 text-xs leading-relaxed text-neutral-400">自动下载回放与谱面包。最近一次仅查询官方近期记录，查不到不会换成最高分；无回放或规则不兼容会明确报错。</p>
    <p v-if="state.replaySelecting" class="text-xs text-neutral-400">正在解析回放...</p>
    <p v-else-if="state.replayFileName" class="flex items-start gap-2 text-xs text-neutral-400">
      <span class="min-w-0 flex-1">{{ state.replayFileName }}<br>可在下方提供匹配的 .osu / .osz；没有谱面包时自动下载。回放保留在浏览器本地。</span>
      <button type="button" :disabled="state.loading" class="shrink-0 font-bold text-[#ff5f45] hover:underline" @click="remove">移除回放</button>
    </p>
    <p v-if="error" role="alert" class="rounded-md border border-red-700/60 bg-red-950/40 p-3 text-xs leading-relaxed text-red-300">{{ error }}</p>
  </fieldset>
</template>
