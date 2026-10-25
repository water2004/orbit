<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { FolderOpen, RefreshCw, ShieldCheck, Trash2 } from "lucide-vue-next";
import { act, workspace, launcher, orbit } from "../services/workspace";
import { savePreferences, type Kind } from "../services/cli";
import { t } from "../i18n";
import Button from "../components/upstream/Button.vue";
import Modal from "../components/Modal.vue";
interface Entry {
  key: string;
  value: unknown;
  explicit?: boolean;
  is_set?: boolean;
  redacted?: boolean;
}
function preferences() {
  if (!workspace.preferences) throw new Error("Preferences are not loaded");
  return workspace.preferences;
}
interface Java {
  runtime_id: string;
  major: number;
  version: string;
  provider: string;
  bytes: number;
  verified?: boolean;
  executable: string;
}
const tab = ref("appearance"),
  entries = reactive<Record<Kind, Entry[]>>({ orbit: [], launcher: [] }),
  values = reactive<Record<string, string>>({}),
  errors = reactive<Record<string, string>>({}),
  runtimes = ref<Java[]>([]),
  minecraft = ref(""),
  removeJava = ref<Java>(),
  clean = ref(false),
  moveTo = ref("");
const labels: Record<string, [string, string]> = {
  "network.concurrency": ["Parallel downloads", "并行下载数量"],
  "network.connect-timeout-seconds": [
    "Connection timeout (seconds)",
    "连接超时（秒）",
  ],
  "network.request-timeout-seconds": [
    "Request timeout (seconds)",
    "请求超时（秒）",
  ],
  "installer.timeout-seconds": [
    "Loader installer timeout (seconds)",
    "加载器安装器超时（秒）",
  ],
  "cache.max-size": ["Artifact cache capacity", "工件缓存容量"],
  "core.default-instance": ["Default instance", "默认实例"],
  "core.max-concurrent-downloads": [
    "Parallel mod downloads",
    "模组并行下载数量",
  ],
  "core.language": ["Terminal language", "终端语言"],
  "network.timeout": ["Network timeout (seconds)", "网络超时（秒）"],
  "network.max-retries": ["Retry limit", "重试次数"],
  "network.proxy": ["Proxy address", "代理地址"],
  "auth.curseforge-api-key": ["CurseForge API key", "CurseForge API 密钥"],
  "auth.modrinth-token": ["Modrinth access token", "Modrinth 访问令牌"],
  "cache.dir": ["JAR cache directory", "JAR 缓存目录"],
  "repository.dir": ["Local version database directory", "本地版本数据库目录"],
  "cache.capacity-mib": ["JAR cache capacity (MiB)", "JAR 缓存容量（MiB）"],
  "ui.progress-bar": ["Terminal progress bars", "终端进度条"],
  "ui.color": ["Terminal colors", "终端颜色"],
};
const choices: Record<string, [string, string, string][]> = {
  "core.language": [
    ["system", "System", "跟随系统"],
    ["en", "English", "English"],
    ["zh-CN", "简体中文", "简体中文"],
  ],
  "ui.progress-bar": [
    ["auto", "Automatic", "自动"],
    ["always", "Always", "始终"],
    ["never", "Never", "从不"],
  ],
  "ui.color": [
    ["auto", "Automatic", "自动"],
    ["always", "Always", "始终"],
    ["never", "Never", "从不"],
  ],
};
const current = computed(
  () => entries[tab.value === "orbit" ? "orbit" : "launcher"],
);
async function load(kind: Kind) {
  try {
    const response = await (kind === "orbit"
      ? orbit<{ entries: Entry[] }>(
          ["config", "list"],
          t("Loading Orbit settings", "正在加载 Orbit 设置"),
          null,
        )
      : launcher<{ settings: Entry[] }>(
          ["config", "list"],
          t("Loading Launcher settings", "正在加载 Launcher 设置"),
          null,
        ));
    entries[kind] =
      "entries" in response ? response.entries : response.settings;
    for (const entry of entries[kind])
      values[kind + entry.key] = entry.key.startsWith("auth.")
        ? ""
        : typeof entry.value === "string"
          ? entry.value
          : entry.value == null
            ? ""
            : String(entry.value);
    errors[kind] = "";
  } catch (e) {
    errors[kind] = String(e);
  }
}
async function java(verify = false) {
  runtimes.value = (
    await launcher<{ runtimes: Java[] }>(
      ["java", "list", ...(verify ? ["--verify"] : [])],
      t("Loading Java runtimes", "正在加载 Java 运行时"),
      null,
    )
  ).runtimes;
}
async function loadMinecraft() {
  const data = await launcher<{ directory: string }>(
    ["minecraft", "directory"],
    t("Loading Minecraft directory", "正在加载 Minecraft 目录"),
    null,
  );
  minecraft.value = data.directory;
}
async function apply(entry: Entry, reset = false) {
  const kind = tab.value === "orbit" ? "orbit" : "launcher";
  const args = reset
    ? ["config", "unset", entry.key]
    : ["config", "set", entry.key, values[kind + entry.key]];
  await (kind === "orbit"
    ? orbit(args, t("Saving setting", "正在保存设置"), null)
    : launcher(args, t("Saving setting", "正在保存设置"), null));
  await load(kind);
}
async function selectBinary(key: "orbit_binary" | "launcher_binary") {
  const path = await open({ multiple: false });
  if (typeof path === "string" && workspace.preferences) {
    workspace.preferences[key] = path;
    await savePreferences(workspace.preferences);
  }
}
onMounted(() => {
  void load("orbit");
  void load("launcher");
  void act(loadMinecraft);
  void act(() => java());
});
</script>
<template>
  <div class="page">
    <header>
      <h1>{{ t("Settings", "设置") }}</h1>
      <p>
        {{ t("Make Orbit feel at home.", "让 Orbit 更适合你的使用习惯。") }}
      </p>
    </header>
    <div class="settings-layout">
      <nav class="settings-nav tabs">
        <button
          v-for="item in [
            ['appearance', t('Appearance', '外观')],
            ['launcher', 'Launcher'],
            ['java', 'Java'],
            ['orbit', 'Orbit'],
          ]"
          :key="item[0]"
          :class="{ active: tab === item[0] }"
          @click="tab = item[0]"
        >
          {{ item[1] }}
        </button>
      </nav>
      <section class="stack settings-content">
        <template v-if="tab === 'appearance' && workspace.preferences">
          <div class="card stack">
            <h2>{{ t("Language & theme", "语言与主题") }}</h2>
            <div class="setting">
              <label>{{ t("Language", "语言") }}</label
              ><select
                v-model="workspace.preferences.language"
                @change="act(() => savePreferences(preferences()))"
              >
                <option value="system">
                  {{ t("Follow system", "跟随系统") }}
                </option>
                <option value="en">English</option>
                <option value="zh-CN">简体中文</option>
              </select>
            </div>
            <div class="setting">
              <label>{{ t("Theme", "主题") }}</label>
              <div class="tabs">
                <button
                  v-for="mode in ['system', 'light', 'dark'] as const"
                  :key="mode"
                  :class="{ active: workspace.preferences.theme_mode === mode }"
                  @click="
                    workspace.preferences.theme_mode = mode;
                    act(() => savePreferences(preferences()));
                  "
                >
                  {{
                    mode === "system"
                      ? t("System", "跟随系统")
                      : mode === "light"
                        ? t("Light", "浅色")
                        : t("Dark", "深色")
                  }}
                </button>
              </div>
            </div>
            <div class="setting">
              <label>{{ t("Accent color", "强调色") }}</label>
              <div class="row">
                <button
                  v-for="color in ['indigo', 'emerald', 'amber'] as const"
                  :key="color"
                  class="swatch"
                  :style="{
                    background: {
                      indigo: '#747bff',
                      emerald: '#42b98a',
                      amber: '#d89c47',
                    }[color],
                  }"
                  :aria-label="color"
                  :aria-pressed="workspace.preferences.accent_theme === color"
                  @click="
                    workspace.preferences.accent_theme = color;
                    act(() => savePreferences(preferences()));
                  "
                />
              </div>
            </div>
          </div>
          <div class="card stack">
            <h2>{{ t("Connected applications", "连接的程序") }}</h2>
            <p>
              {{
                t(
                  "Select the exact CLI executables used by this app.",
                  "选择此界面调用的 CLI 可执行文件。",
                )
              }}
            </p>
            <div
              v-for="key in ['orbit_binary', 'launcher_binary'] as const"
              :key="key"
              class="setting"
            >
              <div class="grow">
                <strong>{{
                  key === "orbit_binary" ? "Orbit" : "Orbit Launcher"
                }}</strong>
                <p class="path">{{ workspace.preferences[key] }}</p>
              </div>
              <Button @click="act(() => selectBinary(key))"
                ><FolderOpen />{{ t("Choose", "选择") }}</Button
              >
            </div>
          </div>
        </template>
        <template v-if="tab === 'launcher' || tab === 'orbit'">
          <p v-if="errors[tab]" class="error">{{ errors[tab] }}</p>
          <div v-if="tab === 'launcher'" class="card stack">
            <h2>{{ t("Minecraft storage", "Minecraft 存储") }}</h2>
            <p class="path">{{ minecraft }}</p>
            <p>
              {{
                t(
                  "Each instance stays isolated. Assets and libraries are shared.",
                  "每个实例保持隔离，资源与库共享存储。",
                )
              }}
            </p>
            <Button
              @click="
                act(async () => {
                  const path = await open({ directory: true });
                  if (typeof path === 'string') moveTo = path;
                })
              "
              ><FolderOpen />{{
                t("Move repository…", "移动存储目录…")
              }}</Button
            >
          </div>
          <div class="card stack">
            <h2>
              {{
                tab === "orbit"
                  ? t("Mod management", "模组管理")
                  : t("Downloads & installation", "下载与安装")
              }}
            </h2>
            <div
              v-for="entry in current.filter((e) => labels[e.key])"
              :key="entry.key"
              class="setting"
            >
              <div class="setting-label">
                <strong>{{ t(...labels[entry.key]) }}</strong
                ><small v-if="entry.key.startsWith('auth.')">{{
                  t(
                    "Existing secrets are not shown. Leave blank to keep them.",
                    "不显示已有密钥，留空不会覆盖。",
                  )
                }}</small>
              </div>
              <select
                v-if="choices[entry.key]"
                v-model="values[tab + entry.key]"
              >
                <option
                  v-for="choice in choices[entry.key]"
                  :key="choice[0]"
                  :value="choice[0]"
                >
                  {{ t(choice[1], choice[2]) }}
                </option></select
              ><input
                v-else
                v-model="values[tab + entry.key]"
                :type="
                  entry.key.startsWith('auth.')
                    ? 'password'
                    : /concurrency|seconds|retries|capacity|downloads|timeout/.test(
                          entry.key,
                        )
                      ? 'number'
                      : 'text'
                "
                :aria-label="t(...labels[entry.key])"
              /><Button
                size="sm"
                :disabled="
                  entry.key.startsWith('auth.') && !values[tab + entry.key]
                "
                @click="act(() => apply(entry))"
                >{{ t("Save", "保存") }}</Button
              ><Button
                type="quiet"
                size="sm"
                @click="act(() => apply(entry, true))"
                >{{ t("Reset", "重置") }}</Button
              >
            </div>
          </div>
          <div v-if="tab === 'orbit'" class="card row">
            <div class="grow">
              <h3>{{ t("JAR cache", "JAR 缓存") }}</h3>
              <p>
                {{
                  t(
                    "Clearing downloaded files does not remove installed mods.",
                    "清理已下载缓存不会移除实例内已安装的模组。",
                  )
                }}
              </p>
            </div>
            <Button @click="clean = true">{{
              t("Clear cache…", "清理缓存…")
            }}</Button>
          </div>
        </template>
        <template v-if="tab === 'java'"
          ><div class="page-heading">
            <div>
              <h2>{{ t("Managed Java runtimes", "托管 Java 运行时") }}</h2>
              <p>
                {{
                  t(
                    "Required Java is installed automatically with the game.",
                    "游戏安装时会自动下载所需 Java。",
                  )
                }}
              </p>
            </div>
            <Button @click="act(() => java(true))"
              ><ShieldCheck />{{ t("Verify all", "全部校验") }}</Button
            >
          </div>
          <div
            v-for="runtime in runtimes"
            :key="runtime.runtime_id"
            class="card stack"
          >
            <div class="row">
              <h3 class="grow">
                Java {{ runtime.major }} · {{ runtime.version }}
              </h3>
              <span v-if="runtime.verified !== null" class="badge">{{
                runtime.verified
                  ? t("Verified", "已校验")
                  : t("Not verified", "未校验")
              }}</span>
            </div>
            <p>
              {{ runtime.provider }} ·
              {{ (runtime.bytes / 1024 / 1024).toFixed(1) }} MiB
            </p>
            <small class="path">{{ runtime.executable }}</small>
            <div class="toolbar">
              <Button
                size="sm"
                @click="
                  act(async () => {
                    await launcher(
                      ['java', 'verify', runtime.runtime_id],
                      t('Verifying Java', '正在校验 Java'),
                      null,
                    );
                    await java();
                  })
                "
                ><RefreshCw />{{ t("Verify", "校验") }}</Button
              ><Button size="sm" @click="removeJava = runtime"
                ><Trash2 />{{ t("Remove", "移除") }}</Button
              >
            </div>
          </div>
          <p v-if="!runtimes.length">
            {{ t("No managed runtimes yet.", "尚无托管运行时。") }}
          </p></template
        >
      </section>
    </div>
    <Modal
      v-if="removeJava"
      :title="t('Remove managed Java?', '移除托管 Java？')"
      @close="removeJava = undefined"
      ><p>Java {{ removeJava.major }} · {{ removeJava.version }}</p>
      <template #footer
        ><Button @click="removeJava = undefined">{{
          t("Cancel", "取消")
        }}</Button
        ><Button
          type="colored"
          @click="
            act(async () => {
              await launcher(
                ['java', 'remove', removeJava!.runtime_id],
                t('Removing Java', '正在移除 Java'),
                null,
              );
              removeJava = undefined;
              await java();
            })
          "
          >{{ t("Remove", "移除") }}</Button
        ></template
      ></Modal
    >
    <Modal
      v-if="clean"
      :title="t('Clear JAR cache?', '清理 JAR 缓存？')"
      @close="clean = false"
      ><p>
        {{
          t(
            "Future operations may need to download these files again.",
            "后续操作可能需要重新下载这些文件。",
          )
        }}
      </p>
      <template #footer
        ><Button @click="clean = false">{{ t("Cancel", "取消") }}</Button
        ><Button
          type="colored"
          @click="
            act(async () => {
              await orbit(
                ['cache', 'clean'],
                t('Clearing cache', '正在清理缓存'),
                null,
              );
              clean = false;
            })
          "
          >{{ t("Clear", "清理") }}</Button
        ></template
      ></Modal
    >
    <Modal
      v-if="moveTo"
      :title="t('Move Minecraft storage?', '移动 Minecraft 存储目录？')"
      @close="moveTo = ''"
      ><p>{{ minecraft }} → {{ moveTo }}</p>
      <template #footer
        ><Button @click="moveTo = ''">{{ t("Cancel", "取消") }}</Button
        ><Button
          type="colored"
          @click="
            act(async () => {
              await launcher(
                ['minecraft', 'move', moveTo],
                t('Moving Minecraft storage', '正在移动 Minecraft 存储'),
                null,
              );
              moveTo = '';
              await loadMinecraft();
            })
          "
          >{{ t("Move", "移动") }}</Button
        ></template
      ></Modal
    >
  </div>
</template>
<style scoped>
.settings-layout {
  display: grid;
  grid-template-columns: 170px minmax(0, 1fr);
  gap: 24px;
}
.settings-nav {
  flex-direction: column;
  align-self: start;
  position: sticky;
  top: 0;
}
.settings-nav button {
  text-align: left;
}
.settings-content {
  max-width: 1000px;
}
.setting {
  display: flex;
  align-items: center;
  gap: 12px;
  flex-wrap: wrap;
  padding: 14px 0;
  border-top: 1px solid var(--surface-4);
}
.setting > label,
.setting-label {
  flex: 1;
  min-width: 180px;
}
.setting-label {
  display: flex;
  flex-direction: column;
  gap: 5px;
}
.setting input {
  width: 180px;
}
.path {
  overflow-wrap: anywhere;
}
.swatch {
  width: 30px;
  height: 30px;
  border-radius: 50%;
  border: 3px solid transparent;
  cursor: pointer;
}
.swatch[aria-pressed="true"] {
  outline: 2px solid var(--color-contrast);
  outline-offset: 3px;
}
</style>
