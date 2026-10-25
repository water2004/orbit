<script setup lang="ts">
// Adapted from Modrinth UI DropdownSelect (GPL-3.0-only); see UPSTREAM.md.
import {
  computed,
  nextTick,
  onMounted,
  onUnmounted,
  ref,
  useId,
  watch,
} from "vue";
import { Check, ChevronDown, Search } from "lucide-vue-next";
import { t } from "../../i18n";
import ProgressSpinner from "./ProgressSpinner.vue";
export interface SelectOption {
  value: string;
  label: string;
  description?: string;
  badge?: string;
  icon?: string;
}
const props = withDefaults(
  defineProps<{
    modelValue?: string | null;
    options: SelectOption[];
    label: string;
    placeholder?: string;
    disabled?: boolean;
    loading?: boolean;
    searchable?: boolean;
  }>(),
  { searchable: true },
);
const emit = defineEmits<{ "update:modelValue": [value: string] }>();
const root = ref<HTMLElement>(),
  trigger = ref<HTMLButtonElement>(),
  input = ref<HTMLInputElement>();
const opened = ref(false),
  query = ref(""),
  active = ref(0),
  above = ref(false),
  available = ref(320);
const id = useId();
const selected = computed(() =>
  props.options.find((option) => option.value === props.modelValue),
);
const filtered = computed(() =>
  props.options.filter((option) =>
    `${option.label} ${option.description || ""}`
      .toLocaleLowerCase()
      .includes(query.value.toLocaleLowerCase()),
  ),
);
function close(restore = false) {
  opened.value = false;
  if (restore) trigger.value?.focus();
}
async function reveal() {
  if (props.disabled || props.loading) return;
  query.value = "";
  active.value = Math.max(
    0,
    props.options.findIndex((option) => option.value === props.modelValue),
  );
  const rect = trigger.value!.getBoundingClientRect();
  const boundary = root.value
    ?.closest(".modal-scroll, .app-content")
    ?.getBoundingClientRect();
  const top = Math.max(0, boundary?.top || 0),
    bottom = Math.min(innerHeight, boundary?.bottom || innerHeight);
  above.value =
    bottom - rect.bottom < 240 && rect.top - top > bottom - rect.bottom;
  available.value = Math.max(
    100,
    Math.min(360, (above.value ? rect.top - top : bottom - rect.bottom) - 12),
  );
  opened.value = true;
  await nextTick();
  if (props.searchable) input.value?.focus();
  scrollActive();
}
function scrollActive() {
  void nextTick(() =>
    root.value
      ?.querySelector<HTMLElement>(`[data-index="${active.value}"]`)
      ?.scrollIntoView?.({ block: "nearest" }),
  );
}
function choose(option?: SelectOption) {
  if (option) {
    emit("update:modelValue", option.value);
    close(true);
  }
}
function keydown(event: KeyboardEvent) {
  if (event.key === "Escape" && opened.value) {
    event.preventDefault();
    event.stopPropagation();
    close(true);
  } else if (["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
    event.preventDefault();
    if (!opened.value) {
      void reveal();
      return;
    }
    const last = filtered.value.length - 1;
    active.value =
      event.key === "Home"
        ? 0
        : event.key === "End"
          ? last
          : Math.max(
              0,
              Math.min(
                last,
                active.value + (event.key === "ArrowDown" ? 1 : -1),
              ),
            );
    scrollActive();
  } else if (event.key === "Enter" && opened.value) {
    event.preventDefault();
    choose(filtered.value[active.value]);
  }
}
function outside(event: PointerEvent) {
  if (!root.value?.contains(event.target as Node)) close();
}
watch(query, () => {
  active.value = 0;
});
watch(
  () => props.disabled || props.loading,
  (value) => {
    if (value) close();
  },
);
onMounted(() => document.addEventListener("pointerdown", outside));
onUnmounted(() => document.removeEventListener("pointerdown", outside));
</script>
<template>
  <div
    ref="root"
    class="dropdown-select"
    @keydown="keydown"
    @focusout="
      (event) => {
        if (!root?.contains(event.relatedTarget as Node)) close();
      }
    "
  >
    <button
      ref="trigger"
      class="select-trigger"
      type="button"
      role="combobox"
      :aria-label="label"
      :aria-expanded="opened"
      :aria-controls="id"
      aria-haspopup="listbox"
      :disabled="disabled || loading"
      :aria-busy="loading || undefined"
      @click="opened ? close() : reveal()"
    >
      <slot name="selected" :option="selected">
        <img v-if="selected?.icon" :src="selected.icon" alt="" />
        <span class="select-copy"
          ><strong>{{
            selected?.label || placeholder || t("Select…", "请选择…")
          }}</strong
          ><small v-if="selected?.description">{{
            selected.description
          }}</small></span
        >
      </slot>
      <ProgressSpinner v-if="loading" /><ChevronDown
        v-else
        :size="17"
        :class="{ rotated: opened }"
      />
    </button>
    <Transition name="popover">
      <div
        v-if="opened"
        class="options-wrapper"
        :class="{ above }"
        :style="{ maxHeight: `${available}px` }"
      >
        <div v-if="searchable" class="select-search">
          <Search :size="16" /><input
            ref="input"
            v-model="query"
            :aria-label="t('Filter options', '筛选选项')"
            :placeholder="t('Search…', '搜索…')"
            :aria-controls="id"
            :aria-activedescendant="
              filtered.length ? `${id}-${active}` : undefined
            "
          />
        </div>
        <div :id="id" role="listbox" :aria-label="label" class="select-options">
          <button
            v-for="(option, index) in filtered"
            :id="`${id}-${index}`"
            :key="option.value"
            type="button"
            role="option"
            tabindex="-1"
            :aria-selected="option.value === modelValue"
            :data-index="index"
            :class="{ active: active === index }"
            @pointermove="active = index"
            @click="choose(option)"
          >
            <img v-if="option.icon" :src="option.icon" alt="" /><span
              class="select-copy"
              ><strong>{{ option.label }}</strong
              ><small v-if="option.description">{{
                option.description
              }}</small></span
            ><span v-if="option.badge" class="badge">{{ option.badge }}</span
            ><Check v-if="option.value === modelValue" :size="16" />
          </button>
          <p v-if="!filtered.length" class="no-options">
            {{ t("No matching options", "没有匹配的选项") }}
          </p>
        </div>
      </div>
    </Transition>
  </div>
</template>
<style scoped>
.dropdown-select {
  position: relative;
  min-width: 0;
}
.select-trigger {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  min-height: 42px;
  padding: 10px 12px;
  border: 1px solid var(--surface-5);
  border-radius: 12px;
  background: var(--surface-3);
  text-align: left;
}
.select-trigger:focus-visible {
  outline: 2px solid var(--color-brand);
  outline-offset: 2px;
}
.select-trigger:disabled {
  opacity: 0.6;
  cursor: default;
}
.select-copy {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 3px;
}
.select-copy strong {
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.select-copy small {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
img {
  width: 28px;
  height: 28px;
  object-fit: contain;
  image-rendering: pixelated;
}
.select-trigger > svg {
  flex-shrink: 0;
  transition: transform var(--motion-fast) var(--motion-ease);
}
.rotated {
  transform: rotate(180deg);
}
.options-wrapper {
  position: absolute;
  z-index: 12;
  top: calc(100% + 6px);
  left: 0;
  right: 0;
  display: flex;
  flex-direction: column;
  min-height: 0;
  padding: 6px;
  border: 1px solid var(--surface-5);
  border-radius: 12px;
  background: var(--surface-2);
  box-shadow: 0 8px 24px #0003;
  transform-origin: top center;
}
.options-wrapper.above {
  top: auto;
  bottom: calc(100% + 6px);
  transform-origin: bottom center;
}
.select-search {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 4px 8px 8px;
}
.select-search input {
  width: 100%;
  min-width: 0;
  background: transparent;
  border: 0;
  box-shadow: none;
  padding: 6px 0;
  outline: 0;
}
.select-options {
  overflow: auto;
  overscroll-behavior: contain;
  min-height: 0;
}
.select-options button {
  width: 100%;
  display: flex;
  gap: 10px;
  align-items: center;
  padding: 10px;
  border-radius: 8px;
  text-align: left;
}
.select-options button.active {
  background: var(--surface-4);
}
.select-options button[aria-selected="true"] {
  color: var(--color-brand);
}
.no-options {
  padding: 12px;
}
</style>
