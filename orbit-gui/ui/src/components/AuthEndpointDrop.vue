<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { useRouter } from "vue-router";
import { t } from "../i18n";
import AuthEndpointDialog from "./AuthEndpointDialog.vue";
import { endpointDialog, openAuthEndpoint } from "../services/auth-endpoint";
import { act } from "../services/workspace";
const router = useRouter();
const dragging = ref(false),
  busy = ref(false);
let localDrag = false;
function over(event: DragEvent) {
  // This is transport detection only. Decoding, URL validation, ALI and provider
  // metadata are exclusively handled by the Launcher CLI.
  if (
    localDrag ||
    busy.value ||
    !event.dataTransfer?.types.includes("text/plain") ||
    event.dataTransfer.types.includes("Files")
  )
    return;
  event.preventDefault();
  event.dataTransfer.dropEffect = "copy";
  dragging.value = true;
}
function drop(event: DragEvent) {
  dragging.value = false;
  if (localDrag) return;
  const raw = event.dataTransfer?.getData("text/plain") || "";
  if (
    event.dataTransfer?.types.includes("Files") ||
    !raw.trim().startsWith("authlib-injector:")
  ) {
    // Unsupported external URLs/files must not navigate the application.
    const target = event.target;
    if (!(
      target instanceof HTMLInputElement ||
      target instanceof HTMLTextAreaElement ||
      (target instanceof HTMLElement && target.isContentEditable)
    ))
      event.preventDefault();
    return;
  }
  event.preventDefault();
  event.stopPropagation();
  if (busy.value) return;
  openAuthEndpoint(raw, endpointDialog.value?.added);
}
function leave(event: DragEvent) {
  if (!event.relatedTarget) dragging.value = false;
}
function start() {
  localDrag = true;
}
function end() {
  localDrag = false;
  dragging.value = false;
}
async function added(id: string) {
  const callback = endpointDialog.value?.added;
  endpointDialog.value = undefined;
  await act(async () => {
    if (callback) await callback(id);
    else await router.push({ path: "/accounts", query: { provider: id } });
  });
}
onMounted(() => {
  window.addEventListener("dragover", over, true);
  window.addEventListener("drop", drop, true);
  window.addEventListener("dragleave", leave, true);
  window.addEventListener("dragstart", start, true);
  window.addEventListener("dragend", end, true);
  window.addEventListener("blur", end);
});
onUnmounted(() => {
  window.removeEventListener("dragover", over, true);
  window.removeEventListener("drop", drop, true);
  window.removeEventListener("dragleave", leave, true);
  window.removeEventListener("dragstart", start, true);
  window.removeEventListener("dragend", end, true);
  window.removeEventListener("blur", end);
});
</script>
<template>
  <Teleport to="body"
    ><Transition name="toast"
      ><div v-if="dragging" class="auth-drop-hint">
        {{ t("Drop to add an authentication service", "放开以添加认证服务") }}
      </div></Transition
    ></Teleport
  >
  <AuthEndpointDialog
    v-if="endpointDialog"
    :key="endpointDialog.key"
    :address="endpointDialog.address"
    @busy="busy = $event"
    @close="endpointDialog = undefined"
    @added="added"
  />
</template>
<style scoped>
.auth-drop-hint {
  position: fixed;
  inset: 16px;
  z-index: 1000;
  pointer-events: none;
  display: grid;
  place-items: center;
  border: 2px dashed var(--color-brand);
  border-radius: 20px;
  background: var(--surface-2);
  color: var(--color-contrast);
  opacity: 0.95;
  font-size: 22px;
  font-weight: 700;
}
</style>
