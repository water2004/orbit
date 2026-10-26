<script setup lang="ts">
import { computed, ref, watch } from "vue";
import Button from "./upstream/Button.vue";
import { t } from "../i18n";
import type { MemoryLimits, MemoryCapabilities } from "../types";
import RangeField from "./upstream/RangeField.vue";
import ProgressSpinner from "./upstream/ProgressSpinner.vue";
const props = defineProps<{
  value: MemoryLimits;
  pending: boolean;
  loading?: boolean;
  capabilities?: MemoryCapabilities;
}>();
const emit = defineEmits<{
  save: [value: MemoryLimits];
  reset: [];
  retry: [];
}>();
const minimum = ref(props.value.min_mib),
  maximum = ref(props.value.max_mib);
watch(
  () => props.value,
  (value) => {
    minimum.value = value.min_mib;
    maximum.value = value.max_mib;
  },
);
const valid = computed(
  () =>
    Number.isInteger(minimum.value) &&
    Number.isInteger(maximum.value) &&
    minimum.value > 0 &&
    maximum.value >= minimum.value,
);
const changed = computed(
  () =>
    minimum.value !== props.value.min_mib ||
    maximum.value !== props.value.max_mib,
);
function preset(value: number) {
  maximum.value = value;
  minimum.value = Math.min(minimum.value, value);
}
</script>
<template>
  <section class="card memory-settings">
    <header>
      <div class="memory-heading">
        <h2>{{ t("Game memory", "游戏内存") }}</h2>
        <span v-if="capabilities" class="memory-system"
          >{{ t("System", "系统内存") }} ·
          {{ (capabilities.physical_memory_mib / 1024).toFixed(1) }} GiB</span
        ><ProgressSpinner v-else-if="loading" :size="16" />
      </div>
      <p>
        {{
          t(
            "Java heap limits for this instance. Changes apply on its next launch.",
            "此实例的 Java 堆内存，修改在下次启动时生效。",
          )
        }}
      </p>
    </header>
    <div v-if="capabilities" class="memory-inputs">
      <RangeField
        v-model="maximum"
        :min="1"
        :max="capabilities.slider_max_mib"
        :label="t('Maximum memory', '最大内存')"
        unit="MiB"
        :disabled="pending"
      />
      <RangeField
        v-model="minimum"
        :min="1"
        :max="capabilities.slider_max_mib"
        :label="t('Initial memory', '初始内存')"
        unit="MiB"
        :disabled="pending"
      />
    </div>
    <Button v-else-if="!loading" type="quiet" @click="emit('retry')">{{
      t("Retry loading memory settings", "重新读取内存设置")
    }}</Button>
    <div class="memory-presets">
      <span>{{ t("Maximum presets", "快捷上限") }}</span>
      <Button
        v-for="size in [2048, 4096, 8192, 16384]"
        :key="size"
        size="sm"
        :disabled="pending"
        :type="maximum === size ? 'colored-text' : 'quiet'"
        @click="preset(size)"
        >{{ size / 1024 }} GiB</Button
      >
    </div>
    <p class="memory-note">
      {{
        t(
          "Minecraft also uses memory outside the heap. Leave room for the operating system and other programs.",
          "Minecraft 还会使用堆外内存，请为系统和其他程序留出空间。",
        )
      }}
    </p>
    <p
      v-if="capabilities && maximum > capabilities.physical_memory_mib"
      class="error"
    >
      {{
        t(
          "This exceeds physical RAM and may prevent startup or cause heavy paging.",
          "此上限超过物理内存，可能无法启动或导致频繁换页。",
        )
      }}
    </p>
    <p v-if="!valid" class="error">
      {{
        t(
          "Enter positive whole MiB values; maximum must not be smaller than initial memory.",
          "请输入正整数 MiB，最大内存不能小于初始内存。",
        )
      }}
    </p>
    <footer>
      <Button
        :disabled="pending || !capabilities || !valid || !changed"
        :loading="pending"
        @click="emit('save', { min_mib: minimum, max_mib: maximum })"
        >{{ t("Save memory settings", "保存内存设置") }}</Button
      >
      <Button type="quiet" :disabled="pending" @click="emit('reset')">{{
        t("Restore defaults", "恢复默认")
      }}</Button>
    </footer>
  </section>
</template>
<style scoped>
.memory-settings {
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.memory-settings header p,
.memory-note {
  margin-top: 4px;
  font-size: 13px;
}
.memory-inputs {
  display: flex;
  flex-direction: column;
  gap: 18px;
  max-width: 700px;
}
.memory-heading {
  display: flex;
  align-items: center;
  gap: 16px;
  flex-wrap: wrap;
}
.memory-system {
  font-size: 12px;
  padding: 4px 8px;
  border-radius: 6px;
  background: var(--surface-4);
}
.memory-presets,
footer {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
}
.memory-presets > span {
  font-size: 12px;
  margin-right: 4px;
}
</style>
