<template>
  <component
    :is="isLink ? RouterLink : 'button'"
    :to="isLink ? to : undefined"
    v-bind="$attrs"
    :disabled="isLink ? undefined : disabled"
    :active-class="isLink && isSubpage ? '' : undefined"
    :class="{
      'router-link-active': isPrimary && isPrimary(route),
      'subpage-active': isSubpage && isSubpage(route),
      disabled: disabled,
    }"
    class="nav-button border-none text-primary cursor-pointer w-12 rounded-full h-12 flex items-center justify-center text-2xl bg-transparent hover:bg-button-bg hover:text-contrast"
    @click="onClick"
  >
    <slot />
  </component>
</template>

<script setup lang="ts">
import { computed } from "vue";
import type { RouteLocationNormalizedLoaded } from "vue-router";
import { RouterLink, useRoute } from "vue-router";

const route = useRoute();

type RouteFunction = (route: RouteLocationNormalizedLoaded) => boolean;

const props = withDefaults(
  defineProps<{
    to: (() => void) | string;
    isPrimary?: RouteFunction;
    isSubpage?: RouteFunction;
    disabled?: boolean;
  }>(),
  {
    disabled: false,
  },
);

const isLink = computed(() => typeof props.to === "string");

function onClick() {
  if (typeof props.to === "function") {
    props.to();
  }
}

defineOptions({
  inheritAttrs: false,
});
</script>

<style lang="scss" scoped>
.nav-button {
  position: relative;
  isolation: isolate;
  transition:
    background-color var(--motion-fast),
    color var(--motion-fast);

  &::before {
    content: "";
    position: absolute;
    inset: 0;
    background-color: var(--color-button-bg-selected);
    border-radius: 50%;
    opacity: 0;
    scale: 0.4;
    z-index: -1;
    transition:
      opacity var(--motion-slow) var(--motion-ease),
      scale var(--motion-slow) var(--motion-ease);
  }
}

.router-link-active {
  @apply text-[--color-button-text-selected];
  &::before {
    background-color: var(--color-button-bg-selected);
    scale: 1;
    opacity: 1;
  }
}

.subpage-active {
  @apply text-contrast bg-button-bg;
}
</style>
