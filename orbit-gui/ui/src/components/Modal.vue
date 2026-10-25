<script setup lang="ts">
import { onMounted, onUnmounted, ref, useId } from "vue";
import { X } from "lucide-vue-next";
import Button from "./upstream/Button.vue";
import { t } from "../i18n";
import type { Task } from "../services/cli";
import TaskProgress from "./TaskProgress.vue";
const props = withDefaults(
  defineProps<{
    title: string;
    wide?: boolean;
    closable?: boolean;
    task?: Task;
  }>(),
  { closable: true },
);
const emit = defineEmits<{ close: [] }>();
const dialog = ref<HTMLDialogElement>();
const heading = ref<HTMLElement>();
const titleId = useId();
const previous = document.activeElement as HTMLElement | null;
onMounted(() => {
  dialog.value?.showModal();
  heading.value?.focus({ preventScroll: true });
});
onUnmounted(() => previous?.focus());
</script>
<template>
  <Teleport to="body"
    ><dialog
      ref="dialog"
      class="modal"
      :aria-labelledby="titleId"
      :class="{ wide }"
      @cancel.prevent="props.closable !== false && emit('close')"
      @click="
        (e) => {
          if (e.target === dialog && props.closable !== false) emit('close');
        }
      "
    >
      <header>
        <h2 :id="titleId" ref="heading" tabindex="-1" style="outline: none">
          {{ title }}
        </h2>
        <Button
          v-if="closable !== false"
          type="quiet"
          :aria-label="t('Close', '关闭')"
          @click="emit('close')"
          ><X
        /></Button>
      </header>
      <section class="modal-scroll"><slot /></section>
      <TaskProgress v-if="task" :task="task" />
      <footer v-if="$slots.footer"><slot name="footer" /></footer></dialog
  ></Teleport>
</template>
