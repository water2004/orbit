<!-- Adapted from Modrinth packages/ui/src/components/base/BigOptionButton.vue.
     GPL-3.0-only. See UPSTREAM.md. -->
<script setup lang="ts">
import type { Component } from "vue";
import { ChevronRight } from "lucide-vue-next";
defineProps<{ icon: Component; title: string; description: string }>();
defineEmits<{ click: [] }>();
</script>
<template>
  <button class="big-option" @click="$emit('click')">
    <span class="option-icon"
      ><component :is="icon" :size="28" :stroke-width="1.5"
    /></span>
    <span class="option-copy"
      ><strong>{{ title }}</strong
      ><small>{{ description }}</small></span
    >
    <ChevronRight :size="20" />
  </button>
</template>
<style scoped>
.big-option {
  display: flex;
  width: 100%;
  align-items: center;
  gap: 16px;
  padding: 16px;
  border-radius: 14px;
  text-align: left;
  background: var(--surface-3);
  border: 1px solid var(--surface-5);
  transition:
    background-color var(--motion-fast),
    border-color var(--motion-fast),
    transform var(--motion-fast) var(--motion-ease);
  cursor: pointer;
}
.big-option:hover {
  background: var(--surface-4);
  border-color: var(--color-brand);
}
.big-option:active {
  transform: scale(0.99);
}
.option-icon {
  display: grid;
  place-items: center;
  width: 56px;
  height: 56px;
  border: 1px solid var(--surface-5);
  border-radius: 12px;
}
.option-copy {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.option-copy strong {
  font-size: 16px;
}
.option-copy small {
  line-height: 1.5;
}
</style>
