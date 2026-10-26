<script setup lang="ts">
import ContentStage from "../components/ContentStage.vue";
import { computed, onMounted, reactive, ref, watch } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { FolderOpen, RefreshCw, ShieldCheck, Trash2 } from "lucide-vue-next";
import { act, workspace, launcher, orbit } from "../services/workspace";
import { savePreferences, type Kind } from "../services/cli";
import { t } from "../i18n";
import Button from "../components/upstream/Button.vue";
import Modal from "../components/Modal.vue";
import JavaChooser from "../components/JavaChooser.vue";
import DropdownSelect from "../components/upstream/DropdownSelect.vue";
import ProgressSpinner from "../components/upstream/ProgressSpinner.vue";
import Toggle from "../components/upstream/Toggle.vue";
import { useCommandScope } from "../services/task-scope";
const {
  task: dialogTask,
  launcher: dialogLauncher,
  orbit: dialogOrbit,
} = useCommandScope();
interface Entry {
  key: string;
  value: unknown;
  explicit?: boolean;
  is_set?: boolean;
  redacted?: boolean;
  sensitive?: boolean;
  allowed_values: string[];
}
function preferences() {
  if (!workspace.preferences) throw new Error("Preferences are not loaded");
  return workspace.preferences;
}
async function changeLanguage(value: string) {
  if (!["system", "en", "zh-CN"].includes(value)) return;
  preferences().language = value as "system" | "en" | "zh-CN";
  await savePreferences(preferences());
}
interface Java {
  runtime_id: string;
  major: number;
  version: string;
  provider: string;
  bytes?: number | null;
  verified?: boolean;
  executable: string;
}
const tab = ref("appearance"),
  javaLoading = ref(false),
  javaError = ref(""),
  savingJava = ref(false),
  entries = reactive<Record<Kind, Entry[]>>({ orbit: [], launcher: [] }),
  values = reactive<Record<string, string>>({}),
  errors = reactive<Record<string, string>>({}),
  saving = reactive(new Set<string>()),
  runtimes = ref<Java[]>([]),
  minecraft = ref(""),
  removeJava = ref<Java>(),
  clean = ref(false),
  moveTo = ref("");
const resolvedPaths = ref<{
  cache_directory: string;
  repository_directory: string;
}>();
const labels: Record<string, [string, string]> = {
  "network.download-source": ["Game download source", "游戏下载源"],
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
function optionLabel(value: string) {
  const labels: Record<string, [string, string]> = {
    system: ["Follow system", "跟随系统"],
    official: ["Official servers", "官方源"],
    bmclapi: ["BMCLAPI · by bangbang93", "BMCLAPI · bangbang93 公益镜像"],
    en: ["English", "English"],
    "zh-CN": ["简体中文", "简体中文"],
    auto: ["Automatic", "自动"],
    always: ["Always", "始终"],
    never: ["Never", "从不"],
    modern: ["Progress bars", "进度条"],
    plain: ["Plain text", "纯文本"],
    off: ["Off", "关闭"],
  };
  return labels[value] ? t(...labels[value]) : value;
}
function placeholder(entry: Entry) {
  if (entry.sensitive)
    return entry.value
      ? t("Configured — enter to replace", "已配置，输入新值以替换")
      : t("Not configured", "未配置");
  if (entry.key === "cache.dir")
    return resolvedPaths.value?.cache_directory || "";
  if (entry.key === "repository.dir")
    return resolvedPaths.value?.repository_directory || "";
  if (entry.key === "network.proxy")
    return t("Direct connection (no proxy)", "直连（未配置代理）");
  return t("Not configured", "未配置");
}
async function chooseDirectory(entry: Entry) {
  const path = await open({ directory: true, multiple: false });
  if (typeof path === "string") values[tab.value + entry.key] = path;
}
const current = computed(
  () => entries[tab.value === "orbit" ? "orbit" : "launcher"],
);
async function load(kind: Kind) {
  try {
    const response = await (kind === "orbit"
      ? orbit<{
          entries: Entry[];
          resolved_paths: {
            cache_directory: string;
            repository_directory: string;
          };
        }>(
          ["config", "list"],
          t("Loading Orbit settings", "正在加载 Orbit 设置"),
          null,
        )
      : launcher<{ settings: Entry[] }>(
          ["config", "list"],
          t("Loading Launcher settings", "正在加载 Launcher 设置"),
          null,
        ));
    const next = "entries" in response ? response.entries : response.settings;
    if (next.some((entry) => !Array.isArray(entry.allowed_values)))
      throw new Error(
        t(
          "The selected CLI does not provide setting definitions. Update its executable in GUI preferences.",
          "所选 CLI 未提供设置定义，请在 GUI 偏好中更新可执行文件。",
        ),
      );
    entries[kind] = next;
    if ("resolved_paths" in response)
      resolvedPaths.value = response.resolved_paths;
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
  if (javaLoading.value) return;
  javaLoading.value = true;
  javaError.value = "";
  try {
    runtimes.value = (
      await launcher<{ runtimes: Java[] }>(
        ["java", "list", ...(verify ? ["--verify"] : [])],
        t("Loading Java runtimes", "正在加载 Java 运行时"),
        null,
      )
    ).runtimes;
  } catch (e) {
    javaError.value = String(e);
  } finally {
    javaLoading.value = false;
  }
}
async function selectJava(value: string | null) {
  if (savingJava.value) return;
  savingJava.value = true;
  try {
    await launcher(
      ["config", "set", "java.selection", value || "auto"],
      t("Saving Java selection", "正在保存 Java 选择"),
      null,
    );
    await load("launcher");
  } finally {
    savingJava.value = false;
  }
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
  const key = kind + entry.key;
  if (saving.has(key)) return;
  saving.add(key);
  try {
    const args = reset
      ? ["config", "unset", entry.key]
      : ["config", "set", entry.key, values[kind + entry.key]];
    await (kind === "orbit"
      ? orbit(args, t("Saving setting", "正在保存设置"), null)
      : launcher(args, t("Saving setting", "正在保存设置"), null));
    await load(kind);
  } finally {
    saving.delete(key);
  }
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
});
watch(tab, (value) => {
  if (value === "java") void act(() => java());
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
      <ContentStage :stage="tab" class="settings-content">
        <template v-if="tab === 'appearance' && workspace.preferences">
          <div class="card stack">
            <h2>{{ t("Language & theme", "语言与主题") }}</h2>
            <div class="setting">
              <label>{{ t("Language", "语言") }}</label
              ><DropdownSelect
                :model-value="workspace.preferences.language"
                :label="t('Language', '语言')"
                :searchable="false"
                :options="[
                  { value: 'system', label: t('Follow system', '跟随系统') },
                  { value: 'en', label: 'English' },
                  { value: 'zh-CN', label: '简体中文' },
                ]"
                @update:model-value="
                  (value) => act(() => changeLanguage(value))
                "
              />
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
                <small
                  v-else-if="
                    entry.value == null &&
                    ['cache.dir', 'repository.dir'].includes(entry.key)
                  "
                  class="path"
                  >{{ t("Default location", "默认位置") }} ·
                  {{ placeholder(entry) }}</small
                >
              </div>
              <DropdownSelect
                v-if="entry.allowed_values.length"
                v-model="values[tab + entry.key]"
                :label="t(...labels[entry.key])"
                :searchable="false"
                :options="
                  entry.allowed_values.map((value) => ({
                    value,
                    label: optionLabel(value),
                  }))
                "
              />
              <input
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
                :placeholder="placeholder(entry)"
              /><Button
                v-if="['cache.dir', 'repository.dir'].includes(entry.key)"
                size="sm"
                :title="t('Choose folder', '选择目录')"
                @click="act(() => chooseDirectory(entry))"
                ><FolderOpen /></Button
              ><Button
                size="sm"
                :loading="saving.has(tab + entry.key)"
                :disabled="
                  !values[tab + entry.key]?.trim() ||
                  (!entry.key.startsWith('auth.') &&
                    values[tab + entry.key] === String(entry.value ?? ''))
                "
                @click="act(() => apply(entry))"
                >{{ t("Save", "保存") }}</Button
              ><Button
                type="quiet"
                size="sm"
                :disabled="saving.has(tab + entry.key)"
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
        <div
          v-if="tab === 'launcher' && workspace.preferences"
          class="card setting"
        >
          <div>
            <h3>
              {{
                t(
                  "Close Launcher after the game starts",
                  "游戏启动后关闭启动器",
                )
              }}
            </h3>
            <p>
              {{
                t(
                  "The game keeps running. Launcher continues collecting logs without the GUI.",
                  "游戏会继续运行，Launcher 在后台收集日志，不依赖 GUI。",
                )
              }}
            </p>
          </div>
          <Toggle
            :model-value="workspace.preferences.close_after_launch"
            @update:model-value="
              (value) => {
                preferences().close_after_launch = value;
                act(() => savePreferences(preferences()));
              }
            "
          />
        </div>
        <div v-if="tab === 'launcher'" class="card">
          <h3>BMCLAPI · by bangbang93</h3>
          <p>
            {{
              t(
                "Mirror metadata and downloads for Minecraft, Fabric and Java. Choosing a mirror trusts its metadata; declared file checksums are still verified. Unsupported components fail explicitly rather than silently contacting official servers. Account authentication and mod providers are unchanged.",
                "Minecraft、Fabric 和 Java 的元数据与资源均走镜像。选择镜像即信任其元数据，文件仍校验已声明的摘要。不支持的组件会明确报错，不静默回源；账户认证与模组 Provider 不受影响。",
              )
            }}
          </p>
          <a
            href="https://bmclapidoc.bangbang93.com/"
            target="_blank"
            rel="noreferrer"
            >{{ t("Service documentation and terms", "服务说明与使用协议") }}</a
          >
        </div>
        <template v-if="tab === 'java'"
          ><div class="page-heading">
            <div>
              <h2>{{ t("Java runtimes", "Java 运行时") }}</h2>
              <p>
                {{
                  t(
                    "Use your installed Java, or let Launcher provision a compatible runtime.",
                    "使用已安装的 Java，或让 Launcher 安装合适的运行时。",
                  )
                }}
              </p>
            </div>
            <Button :loading="javaLoading" @click="act(() => java(true))"
              ><ShieldCheck />{{ t("Verify all", "全部校验") }}</Button
            >
          </div>
          <div class="card stack">
            <h3>{{ t("Default Java selection", "默认 Java 选择") }}</h3>
            <JavaChooser
              :disabled="savingJava || javaLoading"
              :candidates="runtimes"
              :model-value="
                String(
                  entries.launcher.find((e) => e.key === 'java.selection')
                    ?.value || 'auto',
                )
              "
              @update:model-value="(value) => act(() => selectJava(value))"
            />
          </div>
          <p v-if="javaLoading" class="row">
            <ProgressSpinner />{{
              t(
                "Discovering local and managed Java…",
                "正在检测本地与托管 Java…",
              )
            }}
          </p>
          <p v-if="javaError" role="alert" class="error">{{ javaError }}</p>
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
              {{
                runtime.provider === "system"
                  ? t("System Java", "系统 Java")
                  : t("Managed Java", "托管 Java")
              }}
              <template v-if="runtime.bytes != null">
                · {{ (runtime.bytes / 1024 / 1024).toFixed(1) }} MiB</template
              >
            </p>
            <small class="path">{{ runtime.executable }}</small>
            <div v-if="runtime.provider !== 'system'" class="toolbar">
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
          <p v-if="!javaLoading && !javaError && !runtimes.length">
            {{ t("No Java runtimes found.", "未发现 Java 运行时。") }}
          </p></template
        >
      </ContentStage>
    </div>
    <Modal
      v-if="removeJava"
      :task="dialogTask"
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
              await dialogLauncher(
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
      :task="dialogTask"
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
              await dialogOrbit(
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
      :task="dialogTask"
      :title="t('Move Minecraft storage?', '移动 Minecraft 存储目录？')"
      @close="moveTo = ''"
      ><p>{{ minecraft }} → {{ moveTo }}</p>
      <template #footer
        ><Button @click="moveTo = ''">{{ t("Cancel", "取消") }}</Button
        ><Button
          type="colored"
          @click="
            act(async () => {
              await dialogLauncher(
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
