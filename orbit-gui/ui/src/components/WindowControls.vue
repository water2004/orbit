<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { isTauri } from "@tauri-apps/api/core";
import { Minus, Square, Copy, X } from "lucide-vue-next";
import { act } from "../services/workspace";
import { t } from "../i18n";
const maximized = ref(false);
async function sync() {
  if (isTauri()) maximized.value = await getCurrentWindow().isMaximized();
}
async function toggle() {
  if (!isTauri()) return;
  await getCurrentWindow().toggleMaximize();
  await sync();
}
const resized = () => {
  void act(sync);
};
onMounted(() => {
  void act(sync);
  window.addEventListener("resize", resized);
});
onUnmounted(() => window.removeEventListener("resize", resized));
</script>
<template>
  <div class="window-controls">
    <button
      :aria-label="t('Minimize', '最小化')"
      :title="t('Minimize', '最小化')"
      @click="
        act(async () => {
          if (isTauri()) await getCurrentWindow().minimize();
        })
      "
    >
      <Minus :size="16" />
    </button>
    <button
      :aria-label="maximized ? t('Restore', '还原') : t('Maximize', '最大化')"
      :title="maximized ? t('Restore', '还原') : t('Maximize', '最大化')"
      @click="act(toggle)"
    >
      <Copy v-if="maximized" :size="14" /><Square v-else :size="14" />
    </button>
    <button
      class="close-window"
      :aria-label="t('Close window', '关闭窗口')"
      :title="t('Close window', '关闭窗口')"
      @click="
        act(async () => {
          if (isTauri()) await getCurrentWindow().close();
        })
      "
    >
      <X :size="18" />
    </button>
  </div>
</template>
<style scoped>
.window-controls {
  display: flex;
  align-self: stretch;
  margin: -8px -16px -8px 4px;
}
.window-controls button {
  display: grid;
  place-items: center;
  width: 46px;
  border-radius: 0;
  background: transparent;
}
.window-controls button:hover {
  background: var(--surface-4);
}
.window-controls .close-window:hover {
  background: #c42b1c;
  color: white;
}
</style>
