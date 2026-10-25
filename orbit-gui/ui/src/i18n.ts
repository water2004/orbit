import { computed, ref } from "vue";
export const language = ref<"system" | "en" | "zh-CN">("system");
export const chinese = computed(
  () =>
    language.value === "zh-CN" ||
    (language.value === "system" && navigator.language.startsWith("zh")),
);
// Product copy stays at the presentation boundary. CLI messages are localized by the CLI.
export function t(en: string, zh: string) {
  return chinese.value ? zh : en;
}
export function changeLabel(kind: string) {
  const labels: Record<string, [string, string]> = {
    install: ["Install", "安装"],
    upgrade: ["Upgrade", "升级"],
    downgrade: ["Downgrade", "降级"],
    remove: ["Remove", "移除"],
    replace: ["Replace", "替换"],
    keep: ["Keep", "保留"],
    enable: ["Enable", "启用"],
    disable: ["Disable", "禁用"],
  };
  const label = labels[kind];
  return label ? t(...label) : kind;
}
