<!-- Adapted from Modrinth packages/ui/src/components/base/Toggle.vue.
     GPL-3.0-only. See UPSTREAM.md. -->
<script setup lang="ts">
defineProps<{ modelValue: boolean; disabled?: boolean }>();
defineEmits<{ "update:modelValue": [value: boolean] }>();
</script>
<template>
  <button
    type="button"
    role="switch"
    :aria-checked="modelValue"
    :disabled="disabled"
    class="toggle"
    :class="{ enabled: modelValue }"
    @click="$emit('update:modelValue', !modelValue)"
  >
    <span />
  </button>
</template>
<style scoped>
.toggle {
  display: inline-flex;
  align-items: center;
  flex-shrink: 0;
  width: 40px;
  height: 22px;
  padding: 3px;
  border: 1px solid var(--surface-5);
  border-radius: 20px;
  background: var(--surface-4);
  transition: background-color var(--motion-fast);
  cursor: pointer;
}
.toggle span {
  width: 14px;
  height: 14px;
  border-radius: 50%;
  background: var(--color-base);
  transition: transform var(--motion-fast) var(--motion-ease);
}
.toggle.enabled {
  background: var(--color-brand);
  border-color: var(--color-brand);
}
.toggle.enabled span {
  transform: translateX(18px);
  background: var(--color-accent-contrast);
}
.toggle:disabled {
  opacity: 0.5;
  cursor: wait;
}
</style>
