<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { imageUrl } from "../services/cli";
const props = withDefaults(
  defineProps<{
    src?: string;
    alt?: string;
    tintBy?: string;
    size?: string | number;
  }>(),
  { alt: "", size: "100%" },
);
const dimension = computed(() =>
  typeof props.size === "number" ? `${props.size}px` : props.size,
);
const failed = ref(false);
watch(
  () => props.src,
  () => {
    failed.value = false;
  },
);
const hue = computed(
  () =>
    [...(props.tintBy || props.alt || "orbit")].reduce(
      (hash, c) => hash * 31 + c.charCodeAt(0),
      0,
    ) % 360,
);
</script>
<template>
  <div
    class="avatar"
    :style="{ width: dimension, height: dimension, '--avatar-hue': hue }"
  >
    <img
      v-if="src && !failed"
      :src="imageUrl(src)"
      :alt="alt"
      loading="lazy"
      @error="failed = true"
    />
    <span v-else aria-hidden="true">{{
      (alt || tintBy || "O").slice(0, 1).toUpperCase()
    }}</span>
  </div>
</template>
<style scoped>
.avatar {
  display: flex;
  align-items: center;
  justify-content: center;
  overflow: hidden;
  border-radius: inherit;
  background: linear-gradient(
    145deg,
    hsl(var(--avatar-hue) 35% 42%),
    hsl(calc(var(--avatar-hue) + 40) 38% 22%)
  );
  color: white;
  font-size: 2em;
  font-weight: 700;
  flex-shrink: 0;
}
.avatar img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  image-rendering: pixelated;
}
</style>
