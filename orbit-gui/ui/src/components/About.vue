<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { getVersion } from "@tauri-apps/api/app";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  ExternalLink,
  Github,
  RefreshCw,
  Download,
  CircleCheck,
} from "lucide-vue-next";
import { t } from "../i18n";
import { act } from "../services/workspace";
import { Cancelled } from "../services/cli";
import { useCommandScope } from "../services/task-scope";
import Button from "./upstream/Button.vue";
import TaskProgress from "./TaskProgress.vue";

interface UpdateCheck {
  current_version: string;
  status: "update_available" | "up_to_date" | "ahead" | "no_release";
  release: null | {
    version: string;
    name: string;
    url: string;
    published_at: string;
    notes: string;
  };
}
const version = ref("");
const versionError = ref("");
const checking = ref(false);
const error = ref("");
const result = ref<UpdateCheck>();
const { launcher, task } = useCommandScope();
const status = computed(() => {
  switch (result.value?.status) {
    case "update_available":
      return t("A new version is available", "发现新版本");
    case "up_to_date":
      return t("You're up to date", "当前已是最新版本");
    case "ahead":
      return t("You're using a newer build", "当前构建比最新正式版更新");
    case "no_release":
      return t("No release has been published yet", "尚无已发布的正式版本");
    default:
      return t("Check for a new version of Orbit", "检查是否有新版本的 Orbit");
  }
});
onMounted(async () => {
  try {
    version.value = await getVersion();
  } catch (e) {
    versionError.value = String(e);
  }
});
async function check() {
  if (checking.value || !version.value) return;
  checking.value = true;
  error.value = "";
  result.value = undefined;
  try {
    result.value = await launcher<UpdateCheck>(
      ["update", "check", "--current-version", version.value],
      t("Checking for Orbit updates", "正在检查 Orbit 更新"),
      null,
    );
  } catch (e) {
    if (!(e instanceof Cancelled)) error.value = String(e);
  } finally {
    checking.value = false;
  }
}
const repository = "https://github.com/water2004/orbit";
const logo = "/orbit.svg";
</script>

<template>
  <section class="about stack" :aria-label="t('About Orbit', '关于 Orbit')">
    <div class="card about-identity">
      <img :src="logo" alt="" width="72" height="72" />
      <div class="grow">
        <h2>
          Orbit <span v-if="version" class="about-version">{{ version }}</span>
        </h2>
        <p>
          {{
            t(
              "Minecraft instances and mods, in one place.",
              "在一处管理 Minecraft 实例与模组。",
            )
          }}
        </p>
        <small>{{
          t(
            "Made by water2004 and contributors",
            "由 water2004 与贡献者共同开发",
          )
        }}</small>
      </div>
    </div>
    <p v-if="versionError" class="error" role="alert">{{ versionError }}</p>
    <div class="card stack">
      <div class="about-update">
        <div class="grow">
          <h3 class="row">
            <CircleCheck v-if="result?.status === 'up_to_date'" :size="20" />{{
              status
            }}
          </h3>
          <p v-if="result?.release">
            {{ t("Latest release", "最新正式版") }} ·
            {{ result.release.version }}
          </p>
          <p v-else>
            {{
              t(
                "Updates come from GitHub Releases. Nothing is installed automatically.",
                "从 GitHub Releases 检查更新，不会自动安装。",
              )
            }}
          </p>
        </div>
        <Button :loading="checking" :disabled="!version" @click="check"
          ><RefreshCw v-if="!checking" />{{
            t("Check for updates", "检查更新")
          }}</Button
        >
      </div>
      <TaskProgress v-if="checking && task" :task="task" />
      <p v-if="error" class="error" role="alert">
        {{
          t(
            "Couldn't check for updates. Please try again.",
            "未能检查更新，请重试。",
          )
        }}<br />{{ error }}
      </p>
      <template v-if="result?.release">
        <div class="toolbar">
          <Button
            :type="result.status === 'update_available' ? 'colored' : 'base'"
            @click="act(() => openUrl(result!.release!.url))"
            ><Download />{{
              result.status === "update_available"
                ? t("Download update", "下载新版本")
                : t("View release", "查看发布页")
            }}<ExternalLink :size="16"
          /></Button>
        </div>
        <details v-if="result.release.notes" class="about-notes">
          <summary>{{ t("Release notes", "更新说明") }}</summary>
          <div>{{ result.release.notes }}</div>
        </details>
      </template>
    </div>
    <div class="card stack">
      <h3>{{ t("Open source", "开源项目") }}</h3>
      <p>
        {{
          t(
            "Orbit and Launcher are MIT licensed. The desktop app is GPL-3.0; its interface includes components and assets adapted from Modrinth App.",
            "Orbit 与 Launcher 采用 MIT 协议，桌面界面采用 GPL-3.0；部分界面组件与资源改编自 Modrinth App。",
          )
        }}
      </p>
      <div class="toolbar">
        <Button @click="act(() => openUrl(repository))"
          ><Github />{{ t("Source code", "源代码") }}</Button
        >
        <Button @click="act(() => openUrl(repository + '/issues'))"
          >{{ t("Report an issue", "报告问题") }}<ExternalLink :size="16"
        /></Button>
        <Button
          type="quiet"
          @click="
            act(() =>
              openUrl(repository + '/blob/main/orbit-gui/ui/UPSTREAM.md'),
            )
          "
          >{{ t("Licenses & acknowledgements", "许可与致谢")
          }}<ExternalLink :size="16"
        /></Button>
      </div>
    </div>
  </section>
</template>

<style scoped>
.about-identity,
.about-update {
  display: flex;
  align-items: center;
  gap: 20px;
}
.about-identity img {
  flex: none;
}
.about-version {
  font-size: 15px;
  font-weight: 500;
  color: var(--color-base);
  margin-left: 8px;
}
.about-identity small {
  color: var(--color-base);
}
.about-notes {
  border-top: 1px solid var(--surface-4);
  padding-top: 16px;
}
.about-notes summary {
  cursor: pointer;
  font-weight: 600;
}
.about-notes div {
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  margin-top: 12px;
  line-height: 1.65;
  max-height: 360px;
  overflow-y: auto;
}
@media (max-width: 900px) {
  .about-update {
    align-items: flex-start;
    flex-direction: column;
  }
}
</style>
