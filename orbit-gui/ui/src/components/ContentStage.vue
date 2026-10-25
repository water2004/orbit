<script setup lang="ts">
// The key changes only for a navigation step, never for progress updates.
defineOptions({ inheritAttrs: false });
defineProps<{ stage: string | number }>();
</script>
<template>
  <Transition name="content" mode="out-in">
    <div :key="stage" v-bind="$attrs" class="content-stage"><slot /></div>
  </Transition>
</template>
<style scoped>
.content-stage {
  display: flex;
  flex-direction: column;
  gap: 12px;
  min-height: 0;
}
</style>
