<script setup lang="ts">
import { computed, useId } from "vue";
const props = defineProps<{
  modelValue: number;
  min: number;
  max: number;
  label: string;
  unit: string;
  disabled?: boolean;
}>();
const emit = defineEmits<{ "update:modelValue": [value: number] }>();
const id = useId();
const upper = computed(() =>
  Math.max(
    props.max,
    Number.isFinite(props.modelValue) ? props.modelValue : props.min,
  ),
);
const position = computed(
  () =>
    Math.max(
      0,
      Math.min(
        100,
        ((props.modelValue - props.min) /
          Math.max(1, upper.value - props.min)) *
          100,
      ),
    ) || 0,
);
</script>
<template>
  <div class="range-field">
    <label :for="id">{{ label }}</label>
    <div class="range-controls">
      <input
        class="range-track"
        type="range"
        :aria-label="label"
        :min="min"
        :max="upper"
        step="1"
        :value="Number.isFinite(modelValue) ? modelValue : min"
        :disabled="disabled"
        :style="{ '--range-position': `${position}%` }"
        @input="
          emit(
            'update:modelValue',
            ($event.target as HTMLInputElement).valueAsNumber,
          )
        "
      />
      <div class="range-precise">
        <input
          :id="id"
          type="number"
          :min="min"
          step="1"
          :value="modelValue"
          :disabled="disabled"
          @input="
            emit(
              'update:modelValue',
              ($event.target as HTMLInputElement).valueAsNumber,
            )
          "
        /><span>{{ unit }}</span>
      </div>
    </div>
  </div>
</template>
<style scoped>
.range-field {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
label {
  font-size: 13px;
  font-weight: 600;
  color: var(--color-contrast);
}
.range-controls {
  display: flex;
  align-items: center;
  gap: 20px;
}
.range-track {
  appearance: none;
  flex: 1;
  min-width: 0;
  height: 26px;
  border: 0;
  padding: 0;
  box-shadow: none;
  cursor: pointer;
  background: linear-gradient(
      to right,
      var(--color-brand) 0%,
      var(--color-brand) var(--range-position),
      var(--surface-5) var(--range-position),
      var(--surface-5) 100%
    )
    center / 100% 4px no-repeat;
}
.range-track::-webkit-slider-thumb {
  appearance: none;
  width: 16px;
  height: 16px;
  border-radius: 50%;
  background: var(--color-brand);
  border: 2px solid var(--surface-3);
  box-shadow: 0 1px 4px #0003;
}
.range-track::-moz-range-thumb {
  width: 12px;
  height: 12px;
  border-radius: 50%;
  background: var(--color-brand);
  border: 2px solid var(--surface-3);
  box-shadow: 0 1px 4px #0003;
}
.range-track:focus-visible {
  outline: 2px solid var(--color-brand);
  outline-offset: 2px;
}
.range-track:disabled {
  opacity: 0.5;
  cursor: default;
}
.range-precise {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}
.range-precise input {
  width: 96px;
  padding: 7px 9px;
  text-align: right;
  font-variant-numeric: tabular-nums;
}
.range-precise span {
  font-size: 12px;
}
</style>
