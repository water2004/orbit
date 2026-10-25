// Visual regression fixture only. Not an application route or production entry.
import { createApp, h } from "vue";
import ActivityPanel from "../src/components/ActivityPanel.vue";
import { activity } from "../src/services/cli";
import "../src/styles.css";
import "../src/motion.css";
document.documentElement.dataset.theme = "light";
activity.tasks.push({
  id: 1,
  label: "升级模组",
  state: "waiting",
  message: "选择独立方案组 2 / 3",
  interaction: {
    interaction_id: "layout-factor",
    interaction: "resolution",
    prompt: "选择这一组的安装方案",
    allow_cancel: true,
    choices: Array.from({ length: 8 }, (_, i) => ({
      id: String(i),
      label: `方案 ${i + 1}`,
      data: {
        changes: [
          {
            different: false,
            change: {
              package: "fabric-api",
              kind: "upgrade",
              current_version: "0.150.0",
              selected_version: "0.151.0",
            },
          },
          {
            different: true,
            change: {
              package: "sodium",
              kind: "upgrade",
              current_version: "0.9.0",
              selected_version: `0.9.1${i ? "-build." + i : ""}`,
              selected_artifact: `sodium-fabric-0.9.1+mc26.2-build.${i}.jar`,
            },
          },
        ],
      },
    })),
  },
});
createApp({
  render: () =>
    h("div", { style: "height:100dvh;display:flex;flex-direction:column" }, [
      h("main", { class: "page", style: "overflow:auto;min-height:0" }, [
        h("h1", "实例 · 模组内容"),
        ...Array.from({ length: 30 }, (_, i) =>
          h("div", { class: "card" }, "背景内容 " + i),
        ),
      ]),
      h(ActivityPanel),
    ]),
}).mount("#app");
