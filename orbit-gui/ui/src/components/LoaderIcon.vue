<script setup lang="ts">
import { computed } from "vue";
const props = withDefaults(
  defineProps<{ loader?: string | null; size?: number }>(),
  { size: 20 },
);
const src = computed(() =>
  props.loader &&
  ["fabric", "forge", "neoforge", "quilt"].includes(props.loader)
    ? `/upstream/${props.loader}.png`
    : null,
);
</script>
<template>
  <img
    v-if="src"
    :src="src"
    alt=""
    class="loader-icon"
    :style="{ width: `${size}px`, height: `${size}px` }"
  />
</template>
<style scoped>
.loader-icon {
  display: inline-block;
  width: 20px;
  height: 20px;
  object-fit: contain;
  vertical-align: middle;
  margin-right: 4px;
}
</style>
