<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { imageUrl } from "../services/cli";
const props = withDefaults(
  defineProps<{
    src?: string;
    alt?: string;
    tintBy?: string;
    size?: string | number;
    skin?: boolean;
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
    :class="{
      'avatar-placeholder': (!src || failed) && !$slots.fallback,
      'avatar-skin': skin,
    }"
    :style="{ width: dimension, height: dimension, '--avatar-hue': hue }"
  >
    <img
      v-if="src && !failed"
      :src="imageUrl(src)"
      :alt="alt"
      loading="lazy"
      @error="failed = true"
    />
    <slot v-else name="fallback"
      ><span aria-hidden="true">{{
        (alt || tintBy || "O").slice(0, 1).toUpperCase()
      }}</span></slot
    >
  </div>
</template>
<style scoped>
.avatar {
  display: flex;
  align-items: center;
  justify-content: center;
  overflow: hidden;
  border-radius: 10px;
  background: transparent;
  color: white;
  font-size: 2em;
  font-weight: 700;
  flex-shrink: 0;
}
.avatar-placeholder {
  background: linear-gradient(
    145deg,
    hsl(var(--avatar-hue) 35% 42%),
    hsl(calc(var(--avatar-hue) + 40) 38% 22%)
  );
}
.avatar-skin {
  border-radius: 0;
  overflow: visible;
}
.avatar.avatar-skin img {
  object-fit: contain;
}
.avatar img {
  width: 100%;
  height: 100%;
  object-fit: contain;
  image-rendering: pixelated;
}
</style>
