<script setup lang="ts">
import Avatar from "./Avatar.vue";
defineProps<{
  src?: string | null;
  name: string;
  size?: number | string;
  kind?: string;
}>();
</script>
<template>
  <Avatar
    class="instance-icon"
    :src="src || undefined"
    :alt="name"
    :size="size || '100%'"
  >
    <template #fallback
      ><img
        :src="
          kind === 'server'
            ? '/upstream/terminal.png'
            : '/upstream/grass-block.png'
        "
        alt=""
        class="default-icon"
    /></template>
  </Avatar>
</template>
<style scoped>
.instance-icon {
  background: linear-gradient(
    145deg,
    color-mix(in srgb, var(--color-brand) 25%, var(--surface-3)),
    var(--surface-4)
  );
}
.default-icon {
  width: 100%;
  height: 100%;
  object-fit: cover;
}
</style>
