<script setup>
// 通用的单选 chip 组：帧率、清晰度、倍速都用它，避免在手机上点小号下拉框。
defineProps({
  modelValue: { type: [String, Number], required: true },
  options: { type: Array, required: true },
});
const emit = defineEmits(['update:modelValue']);
</script>

<template>
  <div class="flex flex-wrap gap-1.5">
    <button
      v-for="option in options"
      :key="option.value"
      type="button"
      class="rounded-md border px-2.5 py-1 text-xs transition"
      :class="option.value === modelValue
        ? 'border-[#ff5f45] bg-[#ff5f45]/15 text-white'
        : 'border-neutral-700 bg-neutral-900 text-neutral-300 hover:border-neutral-500'"
      @click="emit('update:modelValue', option.value)"
    >
      {{ option.label }}
    </button>
  </div>
</template>
