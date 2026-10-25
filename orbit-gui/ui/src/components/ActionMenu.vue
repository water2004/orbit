<script setup lang="ts">
import { nextTick, onMounted, onUnmounted, ref, useId } from "vue";
import { MoreHorizontal } from "lucide-vue-next";
defineProps<{ label: string }>();
const root = ref<HTMLElement>();
const trigger = ref<HTMLButtonElement>();
const opened = ref(false);
const panelId = useId();
function close(restoreFocus = false) {
  opened.value = false;
  if (restoreFocus) trigger.value?.focus({ preventScroll: true });
}
async function openFirst() {
  opened.value = true;
  await nextTick();
  root.value
    ?.querySelector<HTMLElement>(
      ".menu-panel button:not(:disabled), .menu-panel a[href]",
    )
    ?.focus({ preventScroll: true });
}
function outside(event: PointerEvent) {
  if (!root.value?.contains(event.target as Node)) close();
}
onMounted(() => document.addEventListener("pointerdown", outside));
onUnmounted(() => document.removeEventListener("pointerdown", outside));
</script>
<template>
  <div
    ref="root"
    class="action-menu"
    @keydown.esc.stop.prevent="close(true)"
    @focusout="!root?.contains($event.relatedTarget as Node) && close()"
  >
    <button
      ref="trigger"
      type="button"
      class="menu-trigger"
      :aria-label="label"
      :title="label"
      :aria-expanded="opened"
      :aria-controls="panelId"
      aria-haspopup="true"
      @click="opened = !opened"
      @keydown.down.prevent="openFirst"
    >
      <slot name="trigger"><MoreHorizontal :size="20" /></slot>
    </button>
    <Transition name="popover">
      <div v-if="opened" :id="panelId" class="menu-panel" @click="close()">
        <slot />
      </div>
    </Transition>
  </div>
</template>
<style scoped>
.action-menu {
  position: relative;
}
.menu-trigger {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px;
  border-radius: 8px;
  cursor: pointer;
  border: 0;
  background: transparent;
  transition: background-color var(--motion-fast);
}
.menu-trigger:hover,
.menu-trigger[aria-expanded="true"] {
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
  transform-origin: top right;
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
  transition: background-color var(--motion-fast);
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
