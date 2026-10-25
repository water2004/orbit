<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { MoreHorizontal } from "lucide-vue-next";
defineProps<{ label: string }>();
const root = ref<HTMLDetailsElement>();
function close() {
  if (root.value) root.value.open = false;
}
function outside(event: PointerEvent) {
  if (!root.value?.contains(event.target as Node)) close();
}
onMounted(() => document.addEventListener("pointerdown", outside));
onUnmounted(() => document.removeEventListener("pointerdown", outside));
</script>
<template>
  <details ref="root" class="action-menu" @keydown.esc.stop="close">
    <summary :aria-label="label" :title="label">
      <slot name="trigger"><MoreHorizontal :size="20" /></slot>
    </summary>
    <div class="menu-panel" @click="close"><slot /></div>
  </details>
</template>
<style scoped>
.action-menu {
  position: relative;
}
summary {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px;
  border-radius: 8px;
  cursor: pointer;
  list-style: none;
}
summary::-webkit-details-marker {
  display: none;
}
summary:hover,
details[open] > summary {
  background: var(--surface-4);
}
.menu-panel {
  position: absolute;
  right: 0;
  top: calc(100% + 8px);
  min-width: 260px;
  max-width: min(360px, 85vw);
  padding: 6px;
  background: var(--surface-3);
  border: 1px solid var(--surface-5);
  border-radius: 12px;
  box-shadow: 0 8px 30px #0003;
  z-index: 20;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.menu-panel :deep(button),
.menu-panel :deep(a) {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  padding: 10px;
  border-radius: 8px;
  text-align: left;
  text-decoration: none;
  color: var(--color-contrast);
}
.menu-panel :deep(button:hover),
.menu-panel :deep(a:hover) {
  background: var(--surface-4);
}
.menu-panel :deep(small) {
  display: block;
  font-weight: normal;
}
.menu-panel :deep(hr) {
  border-color: var(--surface-4);
  margin: 4px;
}
</style>
