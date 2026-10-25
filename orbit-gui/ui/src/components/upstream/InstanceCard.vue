<script setup lang="ts">
// Adapted from Modrinth App's library card; see ui/UPSTREAM.md.
import { computed } from "vue";
import Avatar from "../Avatar.vue";
import type { Instance } from "../../types";
const props = withDefaults(
  defineProps<{
    instance: Instance & {
      loader?: string;
      game_version?: string;
      icon_path?: string;
    };
    selected?: boolean;
    compact?: boolean;
  }>(),
  { selected: false, compact: false },
);
const compactMode = computed(() => props.compact);
</script>

<template>
  <div
    class="relative flex w-full min-w-0 select-none overflow-clip border border-solid bg-surface-3 text-left transition-[background-color,border-color,filter]"
    :class="{
      'flex-row items-center justify-start gap-2.5 rounded-xl p-2.5':
        compactMode,
      'flex-col items-start justify-end gap-3 rounded-[20px] p-3': !compactMode,
      '[border-color:color-mix(in_srgb,var(--color-brand)_40%,transparent)] brightness-110':
        selected,
      'border-surface-4': !selected,
    }"
  >
    <div
      class="relative flex shrink-0 items-center max-w-full overflow-clip"
      :class="
        compactMode
          ? 'size-10 rounded-lg'
          : 'aspect-square min-w-full rounded-2xl'
      "
    >
      <Avatar
        class="pointer-events-none outline-none"
        :class="compactMode ? '!rounded-lg' : '!rounded-2xl'"
        size="100%"
        :src="instance.icon_path"
        loading="lazy"
        :tint-by="instance.name"
        :alt="instance.name"
      />
      <slot name="loading" :compact="compactMode" />
      <div
        class="absolute z-[1] flex items-center justify-center"
        :class="compactMode ? 'inset-0' : 'bottom-1.5 right-1.5 size-12'"
      >
        <slot name="leading" :compact="compactMode" />
      </div>
    </div>
    <div
      class="flex min-w-0 w-full flex-col items-start justify-center gap-1 px-0.5"
      :class="{ 'pr-10': compactMode }"
    >
      <p
        :title="instance.name"
        class="m-0 w-full truncate text-base font-semibold leading-5 text-contrast"
      >
        {{ instance.name }}
      </p>
      <p
        :title="instance.game_version"
        class="m-0 w-full truncate text-sm font-medium capitalize leading-[18px] text-primary"
      >
        {{ instance.loader || instance.kind }} {{ instance.game_version || "" }}
      </p>
    </div>
    <slot name="overlay" :compact="compactMode" />
  </div>
</template>
