<script setup lang="ts">
import { computed, inject, onMounted, reactive, ref, watch } from "vue";
import DownloadSourcePicker from "../components/DownloadSourcePicker.vue";
import { useRoute, useRouter } from "vue-router";
import { join } from "@tauri-apps/api/path";
import { open } from "@tauri-apps/plugin-dialog";
import {
  Check,
  ChevronLeft,
  ChevronRight,
  Download,
  FolderOpen,
  ArrowUpCircle,
  Boxes,
  FolderInput,
} from "lucide-vue-next";
import { act, launcher } from "../services/workspace";
import { instanceContextKey } from "../services/instance-context";
import { Cancelled } from "../services/cli";
import {
  createInstallation,
  prepareMigration,
  applyMigration,
  updateLoader,
  type Pack,
  type InstallDraft,
  type Migration,
} from "../services/installations";
import type { MinecraftVersion, LoaderVersion } from "../types";
import { t } from "../i18n";
import { instancePath } from "../services/navigation";
import Button from "../components/upstream/Button.vue";
import BigOptionButton from "../components/upstream/BigOptionButton.vue";
import DropdownSelect from "../components/upstream/DropdownSelect.vue";
import ProgressSpinner from "../components/upstream/ProgressSpinner.vue";
import LoaderIcon from "../components/LoaderIcon.vue";
import ImportInstance from "../components/ImportInstance.vue";
import ContentStage from "../components/ContentStage.vue";
const importOpen = ref(false);
const route = useRoute(),
  router = useRouter(),
  step = ref(route.query.migrate || route.query.loader ? 0 : -1),
  catalog = ref<MinecraftVersion[]>([]),
  loaders = ref<LoaderVersion[]>([]),
  loading = ref(false),
  catalogLoading = ref(false),
  busy = ref(false),
  sourcePending = ref(false),
  versionFilter = ref("release"),
  error = ref(""),
  java = ref<number>(),
  pack = ref<Pack>(),
  optional = ref<string[]>([]),
  review = ref<Migration>();
const sourceContext = inject(instanceContextKey, undefined)?.value;
const source = sourceContext?.state.instance;
const sourceHasOrbit =
  !!sourceContext?.state.detail &&
  sourceContext.state.detail.desired.loader !== "vanilla" &&
  !sourceContext.state.inventoryError;
const mode = computed(() =>
  route.query.migrate ? "migrate" : route.query.loader ? "loader" : "new",
);
const draft = reactive<InstallDraft>({
  name: "",
  kind:
    source?.kind === "server" && mode.value === "migrate" ? "server" : "client",
  minecraft: "",
  loader: "fabric",
  loaderVersion: "",
  serverDirectory: "",
});
const steps = computed(() => [
  t("Game version", "游戏版本"),
  t("Components", "组件"),
  t("Review", "核对安装"),
]);
const visibleVersions = computed(() =>
  catalog.value.filter(
    (v) =>
      versionFilter.value === "all" || v.version_type === versionFilter.value,
  ),
);
const canNext = computed(
  () =>
    !busy.value &&
    !sourcePending.value &&
    (step.value === 0
      ? !catalogLoading.value && !error.value && !!draft.minecraft
      : step.value === 1
        ? !loading.value &&
          !!java.value &&
          !error.value &&
          (draft.loader === "vanilla" || !!draft.loaderVersion)
        : step.value === 2 &&
          !loading.value &&
          !!java.value &&
          !error.value &&
          (draft.loader === "vanilla" || !!draft.loaderVersion) &&
          !!draft.name.trim() &&
          (draft.kind !== "server" || !!draft.serverDirectory)),
);
watch(versionFilter, () => {
  if (!visibleVersions.value.some((version) => version.id === draft.minecraft))
    draft.minecraft = "";
});
let catalogRevision = 0,
  versionsRevision = 0;
async function components(preferredVersion = "") {
  const revision = ++catalogRevision;
  const minecraft = draft.minecraft,
    loader = draft.loader;
  if (!minecraft) {
    loading.value = false;
    return;
  }
  loading.value = true;
  error.value = "";
  loaders.value = [];
  java.value = undefined;
  draft.loaderVersion = "";
  try {
    const [versions, requirement] = await Promise.all([
      loader === "vanilla"
        ? Promise.resolve({ versions: [] as LoaderVersion[] })
        : launcher<{ versions: LoaderVersion[] }>(
            [
              "versions",
              "loader",
              "--loader",
              loader,
              "--minecraft",
              minecraft,
            ],
            t("Loading compatible loaders", "正在加载兼容加载器"),
            null,
          ),
      launcher<{ major: number }>(
        ["versions", "java", "--minecraft", minecraft],
        t("Checking Java requirement", "正在检查 Java 要求"),
        null,
      ),
    ]);
    if (
      revision !== catalogRevision ||
      minecraft !== draft.minecraft ||
      loader !== draft.loader
    )
      return;
    loaders.value = versions.versions;
    draft.loaderVersion =
      pack.value?.loader_version ||
      (versions.versions.some((v) => v.version === preferredVersion)
        ? preferredVersion
        : "") ||
      versions.versions.find((v) => v.stable)?.version ||
      versions.versions[0]?.version ||
      "";
    java.value = requirement.major;
  } catch (e) {
    if (revision === catalogRevision) error.value = String(e);
  } finally {
    if (revision === catalogRevision) loading.value = false;
  }
}
async function inspectPack() {
  const path = await open({
    multiple: false,
    filters: [
      { name: t("Modpack", "整合包"), extensions: ["orbitbundle", "mrpack"] },
    ],
  });
  if (typeof path !== "string") return;
  const data = await launcher<Omit<Pack, "source">>(
    ["package", "inspect", path],
    t("Inspecting modpack", "正在检查整合包"),
    null,
  );
  pack.value = { source: path, ...data };
  draft.name = data.name;
  draft.minecraft = data.minecraft;
  draft.loader = data.loader;
  draft.kind = data.targets.includes("client") ? "client" : "server";
  optional.value = data.optional_files
    .filter((f) => f.targets.includes(draft.kind))
    .map((f) => f.path);
  step.value = 1;
  await components();
}
async function importDirectory() {
  importOpen.value = true;
}
watch(
  () => draft.kind,
  (kind) => {
    if (pack.value)
      optional.value = pack.value.optional_files
        .filter((f) => f.targets.includes(kind))
        .map((f) => f.path);
  },
);
async function next() {
  if (!canNext.value) return;
  if (step.value === 0) {
    step.value = 1;
    await components();
  } else if (step.value === 1) {
    if (!draft.name)
      draft.name = `${draft.minecraft}${draft.loader === "vanilla" ? "" : "-" + draft.loader}`;
    step.value = 2;
  }
}
async function install() {
  if (!canNext.value) return;
  busy.value = true;
  error.value = "";
  try {
    if (mode.value === "loader") {
      if (!source)
        throw new Error(t("Select a source instance", "请选择源实例"));
      await updateLoader(draft.loaderVersion, source, sourceHasOrbit);
      await router.push(instancePath(source.id, "runtime"));
    } else if (mode.value === "migrate") {
      if (!source)
        throw new Error(t("Select a source instance", "请选择源实例"));
      review.value = await prepareMigration({ ...draft }, source);
      await migrate();
    } else {
      const created = await createInstallation(
        { ...draft },
        pack.value,
        optional.value,
      );
      await router.push(instancePath(created.id, "runtime"));
    }
  } catch (e) {
    if (!(e instanceof Cancelled)) error.value = String(e);
  } finally {
    busy.value = false;
  }
}
async function migrate() {
  if (!review.value) return;
  busy.value = true;
  try {
    if (await applyMigration(review.value))
      await router.push(instancePath(review.value.target.id));
  } finally {
    busy.value = false;
  }
}
async function loadCatalog(force = false) {
  if (!force && (catalog.value.length || catalogLoading.value)) return;
  const revision = ++versionsRevision;
  catalogLoading.value = true;
  error.value = "";
  try {
    const result = (
      await launcher<{ versions: MinecraftVersion[] }>(
        ["versions", "minecraft"],
        t("Loading Minecraft versions", "正在加载 Minecraft 版本"),
        null,
      )
    ).versions;
    if (revision !== versionsRevision) return;
    catalog.value = result;
    if (!result.some((version) => version.id === draft.minecraft))
      draft.minecraft = "";
  } catch (e) {
    if (revision === versionsRevision) error.value = String(e);
  } finally {
    if (revision === versionsRevision) catalogLoading.value = false;
  }
}
async function downloadSourceChanged() {
  if (busy.value || review.value || step.value < 0) return;
  if (step.value === 0) await loadCatalog(true);
  else if (draft.minecraft) await components(draft.loaderVersion);
}
function customInstallation() {
  step.value = 0;
}
onMounted(async () => {
  if (mode.value !== "new" && !source) {
    error.value = t(
      "Open a source instance from the library to update or migrate it.",
      "请从实例库打开源实例，再选择更新或迁移。",
    );
    return;
  }
  if (mode.value === "loader" && sourceContext?.state.detail) {
    draft.minecraft = sourceContext.state.detail.desired.minecraft;
    draft.loader = sourceContext.state.detail.desired.loader;
    draft.name = source!.name;
    step.value = 1;
    await components();
  } else if (route.query.pack) await act(inspectPack);
  else if (route.query.import) await act(importDirectory);
  else if (mode.value === "migrate") await loadCatalog();
});
watch(step, (value) => {
  if (value < 1) {
    ++catalogRevision;
    loading.value = false;
    error.value = "";
  }
  if (value === 0) void loadCatalog();
});
</script>
<template>
  <div class="page create-page">
    <ImportInstance
      v-if="importOpen"
      @close="importOpen = false"
      @imported="
        (id) => {
          importOpen = false;
          router.push(instancePath(id, 'runtime'));
        }
      "
    />
    <header class="page-heading">
      <div>
        <h1>
          {{
            mode === "migrate"
              ? t("Move to a new version", "迁移到新版本")
              : mode === "loader"
                ? t("Update your loader", "更新加载器")
                : t("Create an instance", "创建实例")
          }}
        </h1>
        <p>
          {{
            mode === "migrate"
              ? t(
                  "Your original installation stays untouched.",
                  "原实例保持不变。",
                )
              : t(
                  "Choose your game, add a loader, and make it yours.",
                  "选择游戏版本与加载器，开始构建你的实例。",
                )
          }}
        </p>
        <p v-if="source && mode !== 'new'" class="source-context">
          <strong
            >{{ t("Source instance", "源实例") }} · {{ source.name }}</strong
          >
        </p>
      </div>
      <div class="heading-actions">
        <DownloadSourcePicker
          v-if="step >= 0 && !review"
          :disabled="busy"
          @changed="downloadSourceChanged"
          @pending="sourcePending = $event"
        />
        <Button
          size="sm"
          :disabled="busy"
          @click="
            router.push(
              source && mode !== 'new'
                ? instancePath(source.id, 'runtime')
                : '/',
            )
          "
          >{{ t("Cancel", "取消") }}</Button
        >
      </div>
    </header>
    <nav v-if="!review && step >= 0" class="stepper">
      <div
        v-for="(label, index) in steps"
        :key="index"
        :class="{ active: step === index, complete: step > index }"
      >
        <span
          ><Check v-if="step > index" :size="15" /><template v-else>{{
            index + 1
          }}</template></span
        >{{ label }}
      </div>
    </nav>
    <p v-if="error" class="error card">{{ error }}</p>
    <ContentStage :stage="review ? 'migration' : step">
      <div v-if="step === -1" class="setup-options stack">
        <h2>{{ t("How would you like to start?", "从哪里开始？") }}</h2>
        <BigOptionButton
          :icon="Boxes"
          :title="t('Build your own', '创建自己的实例')"
          :description="
            t(
              'Choose Minecraft and a loader, then add the mods you want.',
              '选择 Minecraft 和加载器，再添加你需要的模组。',
            )
          "
          @click="customInstallation"
        />
        <BigOptionButton
          :icon="Download"
          :title="t('Install a modpack', '从整合包安装')"
          :description="
            t(
              'Open an Orbit bundle or Modrinth .mrpack file.',
              '打开 Orbit 组合包或 Modrinth .mrpack 文件。',
            )
          "
          @click="act(inspectPack)"
        />
        <BigOptionButton
          :icon="FolderInput"
          :disabled="busy"
          :title="t('Import an existing instance', '导入已有实例')"
          :description="
            t(
              'Copy an existing launcher instance into Orbit, keeping your original files.',
              '将其他启动器的实例导入 Orbit，保留原文件。',
            )
          "
          @click="act(importDirectory)"
        />
      </div>
      <template v-if="review"
        ><div class="card stack">
          <h2>{{ t("Migration target", "迁移目标") }}</h2>
          <p>{{ source?.name }} → {{ review.target.name }}</p>
          <p>
            {{
              t(
                "The target runtime is ready. Orbit selects and confirms the mod migration in one operation.",
                "目标运行时已准备好。Orbit 将在同一次操作中完成模组方案选择与最终确认。",
              )
            }}
          </p>
        </div>
        <div class="toolbar">
          <p v-if="!review.exported" class="grow">
            {{
              t(
                "The source snapshot is kept if you stop here.",
                "若在此停止，迁移源快照会保留。",
              )
            }}<br /><small>{{ review.sourcePack }}</small>
          </p>
          <p v-else class="grow">
            {{
              t(
                "The selected plan is saved in the target. Continue to install its exact packages without selecting again.",
                "所选方案已保存到目标实例。继续将安装精确包版本，无需重新选择。",
              )
            }}
          </p>
          <Button type="colored" :loading="busy" @click="act(migrate)">{{
            t("Continue migration", "继续迁移")
          }}</Button>
        </div></template
      >
      <template v-else-if="step >= 0">
        <div v-if="step === 0" class="stack">
          <div class="toolbar">
            <div class="tabs">
              <button
                v-for="item in [
                  ['release', t('Releases', '正式版')],
                  ['snapshot', t('Snapshots', '快照')],
                  ['all', t('All versions', '全部版本')],
                ]"
                :key="item[0]"
                :class="{ active: versionFilter === item[0] }"
                @click="versionFilter = item[0]"
              >
                {{ item[1] }}
              </button>
            </div>
          </div>
          <label class="field"
            >{{ t("Minecraft version", "Minecraft 版本")
            }}<DropdownSelect
              v-model="draft.minecraft"
              :label="t('Minecraft version', 'Minecraft 版本')"
              :loading="catalogLoading"
              :options="
                visibleVersions.map((version) => ({
                  value: version.id,
                  label: version.id,
                  badge:
                    version.version_type === 'release'
                      ? t('Release', '正式版')
                      : version.version_type === 'snapshot'
                        ? t('Snapshot', '快照')
                        : version.version_type,
                }))
              "
          /></label>
          <Button
            v-if="!catalogLoading && (error || !catalog.length)"
            @click="loadCatalog(true)"
            >{{ t("Retry", "重试") }}</Button
          >
        </div>
        <div v-if="step === 1" class="stack">
          <div class="card stack components-card">
            <h2>Minecraft {{ draft.minecraft }}</h2>
            <p>
              {{
                t(
                  "Only versions compatible with this Minecraft release are listed.",
                  "这里只列出兼容此 Minecraft 版本的加载器版本。",
                )
              }}
            </p>
            <div class="loader-grid">
              <button
                v-for="loader in [
                  'vanilla',
                  'fabric',
                  'quilt',
                  'forge',
                  'neoforge',
                ]"
                :key="loader"
                :disabled="mode === 'loader' || !!pack"
                :class="{ selected: draft.loader === loader }"
                :aria-pressed="draft.loader === loader"
                @click="
                  draft.loader = loader;
                  act(components);
                "
              >
                <LoaderIcon
                  v-if="loader !== 'vanilla'"
                  :loader="loader"
                  :size="20"
                />
                <strong>{{
                  loader === "vanilla"
                    ? t("Vanilla", "原版")
                    : loader === "neoforge"
                      ? "NeoForge"
                      : loader[0].toUpperCase() + loader.slice(1)
                }}</strong
                ><Check v-if="draft.loader === loader" :size="16" />
              </button>
            </div>
            <label
              v-if="draft.loader !== 'vanilla'"
              class="field loader-version"
              >{{ t("Loader version", "加载器版本")
              }}<DropdownSelect
                v-model="draft.loaderVersion"
                :disabled="!!pack"
                :loading="loading"
                :label="t('Loader version', '加载器版本')"
                :options="
                  loaders.map((version) => ({
                    value: version.version,
                    label: version.version,
                    badge: version.stable ? t('Stable', '稳定版') : undefined,
                  }))
                "
            /></label>
            <div class="java-requirement row">
              <div class="grow">
                <h3>Java {{ java ?? "—" }}</h3>
                <p>
                  {{
                    t(
                      "Launcher follows your Java selection, preferring a matching local runtime in automatic mode.",
                      "Launcher 遵循 Java 选择；自动模式优先使用匹配的本地运行时。",
                    )
                  }}
                </p>
              </div>
              <ProgressSpinner v-if="loading" /><span v-else class="badge">{{
                t("Automatic selection", "按设置选择")
              }}</span>
            </div>
          </div>
          <Button v-if="!loading && error" @click="components()">{{
            t("Retry loading components", "重新加载组件")
          }}</Button>
        </div>
        <div v-if="step === 2" class="stack">
          <div class="card stack">
            <label class="field"
              >{{ t("Instance name", "实例名称")
              }}<input v-model="draft.name" :disabled="mode === 'loader'"
            /></label>
            <div class="row">
              <label
                ><input
                  v-model="draft.kind"
                  type="radio"
                  value="client"
                  :disabled="
                    mode !== 'new' ||
                    (!!pack && !pack.targets.includes('client'))
                  "
                />{{ t("Client", "客户端") }}</label
              ><label
                ><input
                  v-model="draft.kind"
                  type="radio"
                  value="server"
                  :disabled="
                    mode !== 'new' ||
                    (!!pack && !pack.targets.includes('server'))
                  "
                />{{ t("Dedicated server", "独立服务端") }}</label
              >
            </div>
            <div v-if="draft.kind === 'server'" class="row">
              <span class="grow">{{
                draft.serverDirectory ||
                t("Choose a new server directory", "选择新的服务端目录")
              }}</span
              ><Button
                @click="
                  act(async () => {
                    const path = await open({ directory: true });
                    if (typeof path === 'string')
                      draft.serverDirectory = await join(path, draft.name);
                  })
                "
                ><FolderOpen />{{
                  t("Choose parent folder", "选择存放位置")
                }}</Button
              >
            </div>
            <div class="review-grid">
              <span>Minecraft</span><strong>{{ draft.minecraft }}</strong
              ><span>{{ t("Loader", "加载器") }}</span
              ><strong>{{ draft.loader }} {{ draft.loaderVersion }}</strong
              ><span>Java</span><strong>{{ java }}</strong
              ><template v-if="pack"
                ><span>{{ t("Modpack", "整合包") }}</span
                ><strong>{{ pack.name }} {{ pack.version }}</strong></template
              >
            </div>
          </div>
          <div v-if="pack?.optional_files.length" class="card stack">
            <h3>{{ t("Optional content", "可选内容") }}</h3>
            <label
              v-for="file in pack.optional_files.filter((f) =>
                f.targets.includes(draft.kind),
              )"
              :key="file.path"
              ><input v-model="optional" type="checkbox" :value="file.path" />{{
                file.path
              }}</label
            >
          </div>
          <p v-if="mode === 'migrate'">
            {{
              t(
                "The source is exported first. The new runtime is then installed, and Orbit checks mod compatibility against the actual target.",
                "先导出源实例，再安装目标运行时，随后由 Orbit 针对实际目标检查模组兼容性。",
              )
            }}
          </p>
        </div>
        <footer class="wizard-footer">
          <Button
            :disabled="
              busy ||
              (step === 0 && mode !== 'new') ||
              (step === 1 && (mode === 'loader' || !!pack))
            "
            @click="step--"
            ><ChevronLeft />{{ t("Back", "上一步") }}</Button
          ><span class="grow" /><Button
            v-if="step < 2"
            type="colored"
            :disabled="!canNext || loading"
            @click="act(next)"
            >{{ t("Continue", "继续") }}<ChevronRight /></Button
          ><Button
            v-else
            type="colored"
            :disabled="!canNext"
            :loading="busy"
            @click="install"
            ><ArrowUpCircle v-if="mode !== 'new'" /><Download v-else />{{
              mode === "migrate"
                ? t("Create target & check", "创建目标并检查")
                : mode === "loader"
                  ? t("Update loader", "更新加载器")
                  : t("Install", "安装")
            }}</Button
          >
        </footer>
      </template>
    </ContentStage>
  </div>
</template>
<style scoped>
.create-page {
  max-width: 900px;
  margin-inline: auto;
  min-height: 0;
  gap: 20px;
}
.heading-actions {
  display: flex;
  align-items: center;
  gap: 8px;
}
@media (max-width: 760px) {
  .page-heading {
    flex-wrap: wrap;
  }
  .heading-actions {
    width: 100%;
    justify-content: flex-end;
  }
}
.setup-options {
  max-width: 720px;
  width: 100%;
  margin: 12px auto;
  gap: 16px;
}
.stepper {
  display: flex;
  gap: 28px;
  padding: 10px 0;
}
.stepper > div {
  display: flex;
  align-items: center;
  gap: 8px;
  transition: color var(--motion-fast);
}
.stepper span {
  width: 27px;
  height: 27px;
  display: grid;
  place-items: center;
  border-radius: 50%;
  background: var(--surface-4);
  transition:
    background-color var(--motion-normal),
    color var(--motion-normal);
}
.stepper .active {
  color: var(--color-contrast);
  font-weight: 700;
}
.stepper .active span,
.stepper .complete span {
  background: var(--color-brand);
  color: var(--color-accent-contrast);
}
.selected {
  background: var(--color-button-bg-selected) !important;
  box-shadow: inset 0 0 0 1px var(--color-brand);
  color: var(--color-contrast);
}
.loader-grid {
  display: grid;
  grid-template-columns: repeat(5, 1fr);
  gap: 8px;
}
.loader-grid button {
  border: 1px solid var(--surface-5);
  border-radius: 12px;
  padding: 10px;
  display: flex;
  align-items: center;
  gap: 7px;
  background: var(--surface-2);
  cursor: pointer;
  transition:
    background-color var(--motion-fast),
    border-color var(--motion-fast),
    box-shadow var(--motion-fast);
}
.loader-grid strong {
  flex: 1;
  text-align: left;
  font-size: 13px;
}
.loader-version {
  width: min(360px, 100%);
  margin-top: 4px;
}
.java-requirement {
  border-top: 1px solid var(--surface-4);
  padding-top: 14px;
  margin-top: 4px;
}
.java-requirement p {
  font-size: 13px;
  margin-top: 4px;
}
.components-card {
  gap: 14px;
}
.loader-grid button:disabled {
  cursor: default;
  opacity: 0.6;
}
.review-grid {
  display: grid;
  grid-template-columns: 140px 1fr;
  gap: 12px;
  padding-top: 15px;
  border-top: 1px solid var(--surface-4);
}
.wizard-footer {
  display: flex;
  align-items: center;
  gap: 12px;
  background: var(--surface-1);
  padding: 16px 0;
  margin-top: 8px;
  border-top: 1px solid var(--surface-4);
}
@media (max-width: 760px) {
  .loader-grid {
    grid-template-columns: repeat(3, 1fr);
  }
}
</style>
