<script setup lang="ts">
// Circle treatment adapted from Modrinth UI's ProgressSpinner (GPL-3.0-only).
withDefaults(defineProps<{ size?: number }>(), { size: 18 });
</script>
<template>
  <svg
    class="progress-spinner"
    :width="size"
    :height="size"
    viewBox="0 0 24 24"
    aria-hidden="true"
  >
    <circle class="track" cx="12" cy="12" r="9" />
    <circle class="arc" cx="12" cy="12" r="9" />
  </svg>
</template>
<style scoped>
.progress-spinner {
  flex-shrink: 0;
  animation: spinner 1s linear infinite;
}
circle {
  fill: none;
  stroke: currentColor;
  stroke-width: 2.5;
}
.track {
  opacity: 0.18;
}
.arc {
  stroke-dasharray: 38 57;
  stroke-linecap: round;
}
@keyframes spinner {
  to {
    transform: rotate(360deg);
  }
}
@media (prefers-reduced-motion: reduce) {
  .progress-spinner {
    animation-duration: 3s;
  }
}
</style>
