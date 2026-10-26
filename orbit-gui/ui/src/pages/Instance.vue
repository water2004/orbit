<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import {
  FolderOpen,
  Play,
  ArrowUpCircle,
  Settings2,
  Trash2,
  PackagePlus,
  Download,
  Upload,
  ShieldCheck,
  RefreshCw,
  ArrowLeft,
} from "lucide-vue-next";
import {
  workspace,
  orbit,
  launcher,
  mutate,
  refresh,
  selectInstance,
  launch,
  act,
} from "../services/workspace";
import { importArguments } from "../services/commands";
import { initializeOrbit } from "../services/installations";
import { activity } from "../services/cli";
import { launchDiagnostics } from "../services/launch-diagnostics";
import type { Package, MemoryLimits, MemorySettingsResult } from "../types";
import MemorySettings from "../components/MemorySettings.vue";
import { t } from "../i18n";
import { instancePath, discoveryPath } from "../services/navigation";
import Avatar from "../components/Avatar.vue";
import Button from "../components/upstream/Button.vue";
import Modal from "../components/Modal.vue";
import PackageManager from "../components/PackageManager.vue";
import ActionMenu from "../components/ActionMenu.vue";
import Toggle from "../components/upstream/Toggle.vue";
import JavaChooser from "../components/JavaChooser.vue";
import InstanceIcon from "../components/InstanceIcon.vue";
import { useCommandScope } from "../services/task-scope";
import DropdownSelect from "../components/upstream/DropdownSelect.vue";
const {
  task: dialogTask,
  orbit: dialogOrbit,
  launcher: dialogLauncher,
  mutate: dialogMutate,
} = useCommandScope();
const exporting = ref(false),
  importing = ref(false);
const launchReportNote = ref("");
const memorySaving = ref(false);
const memoryLoading = ref(false),
  memorySettings = ref<MemorySettingsResult>();
let memoryRevision = 0;
async function loadMemory() {
  const instance = workspace.selected;
  if (!instance) return;
  const revision = ++memoryRevision;
  memoryLoading.value = true;
  memorySettings.value = undefined;
  try {
    const result = await launcher<MemorySettingsResult>(
      ["instance", "memory", "show"],
      t("Loading instance memory", "正在读取实例内存"),
      instance,
    );
    if (revision === memoryRevision && workspace.selected?.id === instance.id)
      memorySettings.value = result;
  } finally {
    if (revision === memoryRevision) memoryLoading.value = false;
  }
}
async function saveMemory(value?: MemoryLimits) {
  const instance = workspace.selected;
  if (!instance || memorySaving.value) return;
  memorySaving.value = true;
  try {
    const result = await launcher<MemorySettingsResult>(
      value
        ? [
            "instance",
            "memory",
            "set",
            "--min-mib",
            String(value.min_mib),
            "--max-mib",
            String(value.max_mib),
          ]
        : ["instance", "memory", "clear"],
      t("Updating instance memory", "正在更新实例内存"),
      instance,
    );
    if (workspace.selected?.id === instance.id) {
      memorySettings.value = result;
      await selectInstance(instance);
    }
  } finally {
    memorySaving.value = false;
  }
}
async function showLaunchReport() {
  const record = await launcher<{ result: unknown | null } | null>(
    ["instance", "launch-report"],
    t("Loading launch report", "正在读取启动报告"),
  );
  const diagnostics = launchDiagnostics(record?.result);
  if (diagnostics) activity.diagnostics = diagnostics;
  else
    launchReportNote.value = !record
      ? t("No launch record exists yet.", "尚无启动记录。")
      : record.result === null
        ? t(
            "The latest launch has not recorded an exit yet.",
            "最近一次启动尚未记录退出结果。",
          )
        : t(
            "No crash report is available for the latest launch.",
            "最近一次启动没有可用的崩溃报告。",
          );
}
const route = useRoute(),
  router = useRouter(),
  query = ref(""),
  manager = ref<Package>(),
  checking = ref(false),
  showUpdates = ref(false);
const maintenanceOpen = ref(false);
const configuringJava = ref(false);
const changingIcon = ref(false);
async function changeJava(value: string | null) {
  const instance = selected();
  configuringJava.value = true;
  try {
    await launcher(
      ["instance", "java", ...(value == null ? ["clear"] : ["set", value])],
      t("Saving Java selection", "正在保存 Java 选择"),
      instance,
    );
    if (workspace.selected?.id === instance.id) await selectInstance(instance);
  } finally {
    configuringJava.value = false;
  }
}
async function changeIcon(reset = false) {
  if (changingIcon.value) return;
  changingIcon.value = true;
  try {
    const instance = selected();
    const image = reset
      ? null
      : await open({
          multiple: false,
          filters: [
            {
              name: t("Image", "图片"),
              extensions: ["png", "jpg", "jpeg", "webp"],
            },
          ],
        });
    if (!reset && typeof image !== "string") return;
    await launcher(
      ["instance", "icon", ...(reset ? ["clear"] : ["set", image as string])],
      t("Updating instance icon", "正在更新实例图标"),
      instance,
    );
    await refresh(instance.id);
  } finally {
    changingIcon.value = false;
  }
}
const activating = ref(new Set<string>());
async function togglePackage(pkg: Package) {
  if (activating.value.has(pkg.mod_id)) return;
  activating.value.add(pkg.mod_id);
  try {
    await mutate(
      [pkg.enabled ? "disable" : "enable", pkg.mod_id],
      t("Updating activation", "正在更新启用状态"),
    );
  } finally {
    activating.value.delete(pkg.mod_id);
  }
}
const openPath = (path: string) => invoke("open_directory", { path });
function selected() {
  if (!workspace.selected)
    throw new Error(t("Select an instance", "请选择实例"));
  return workspace.selected;
}
const renameOpen = ref(false),
  newName = ref(""),
  unregister = ref(false),
  deleteOpen = ref(false),
  deleting = ref(false),
  deletion = ref<{ name: string; directory: string }>();
const tab = computed(() => String(route.params.tab || "mods"));
watch(
  [tab, () => workspace.selected?.id],
  () => {
    ++memoryRevision;
    memorySettings.value = undefined;
    if (tab.value === "runtime") void act(loadMemory);
  },
  { immediate: true },
);
interface Updates {
  updates: { mod_id: string; current_version: string; new_version: string }[];
  diagnostics: {
    package: string;
    candidate_version: string;
    facts: string[];
  }[];
  warnings: string[];
}
const updates = ref<Updates>(),
  audit = ref<{
    readiness_message: string;
    artifacts: number;
    runtime_namespace?: string;
    findings: {
      packages: string;
      rule: string;
      reason: string;
      risk: number;
      severity: string;
      confidence: string;
    }[];
    warnings: { detail: string }[];
    coverage_gaps: { detail: string; count: number }[];
  }>();
watch(
  () => workspace.packages,
  () => {
    updates.value = undefined;
  },
);
const minRisk = ref("35"),
  auditFilter = ref(""),
  auditBusy = ref(false);
const packages = computed(() =>
  workspace.packages.filter(
    (p) =>
      p.mod_id.toLowerCase().includes(query.value.toLowerCase()) &&
      (!showUpdates.value ||
        updates.value?.updates.some((u) => u.mod_id === p.mod_id)),
  ),
);
const exportOpen = ref(false),
  format = ref("orbit"),
  includeData = ref(false),
  includeRuntime = ref(false);
const importReview = ref<{
  path: string;
  optional: string[];
  selected: string[];
}>();
const width = ref(1280),
  height = ref(720),
  serverCommand = ref(""),
  serverStatus = ref<{ running: boolean }>(),
  eula = ref<{ text: string; digest_sha256: string; url: string }>();
watch(
  () => workspace.selected?.id,
  () => {
    updates.value = undefined;
    audit.value = undefined;
    manager.value = undefined;
    serverStatus.value = undefined;
    eula.value = undefined;
    importReview.value = undefined;
    exportOpen.value = false;
    unregister.value = false;
    deleteOpen.value = false;
    deletion.value = undefined;
    renameOpen.value = false;
  },
);
watch(
  () => workspace.detail,
  (detail) => {
    width.value = detail?.client_resolution?.width ?? 1280;
    height.value = detail?.client_resolution?.height ?? 720;
  },
);
async function initializeMods() {
  const d = workspace.detail;
  if (!d) return;
  await initializeOrbit(d);
  if (workspace.selected?.id === d.id) await selectInstance(d);
}
async function reviewDeletion() {
  deletion.value = await dialogLauncher(
    ["instance", "delete", "--dry-run"],
    t("Reviewing instance deletion", "正在检查实例删除范围"),
    selected(),
  );
  deleteOpen.value = true;
}
async function deleteInstance() {
  deleting.value = true;
  try {
    await dialogLauncher(
      ["instance", "delete", "--yes"],
      t("Deleting entire instance", "正在删除整个实例"),
      selected(),
    );
    deleteOpen.value = false;
    await refresh();
    await router.push("/");
  } finally {
    deleting.value = false;
  }
}
async function checkUpdates() {
  const selected = workspace.selected;
  checking.value = true;
  try {
    const result = await orbit<Updates>(
      ["outdated"],
      t("Checking mod updates", "正在检查模组更新"),
      selected,
    );
    if (workspace.selected?.id === selected?.id) {
      updates.value = result;
      showUpdates.value = true;
    }
  } finally {
    checking.value = false;
  }
}
async function runAudit(report?: string) {
  const selected = workspace.selected;
  auditBusy.value = true;
  try {
    const result = await orbit<NonNullable<typeof audit.value>>(
      [
        "audit",
        "--min-risk",
        minRisk.value,
        ...(auditFilter.value ? ["--mod", auditFilter.value] : []),
        ...(report ? ["--report", report] : []),
      ],
      t("Auditing compatibility", "正在审计兼容性"),
      selected,
    );
    if (workspace.selected?.id === selected?.id) audit.value = result;
  } finally {
    auditBusy.value = false;
  }
}
async function exportPack() {
  if (exporting.value) return;
  const instance = workspace.selected;
  if (!instance) return;
  const selectedFormat = format.value;
  const content = includeData.value ? "mods-and-data" : "mods";
  const withRuntime = selectedFormat === "orbit" && includeRuntime.value;
  const path = await save({
    defaultPath: `${instance.name}.${selectedFormat === "orbit" ? "orbitbundle" : "mrpack"}`,
  });
  if (!path) return;
  exporting.value = true;
  try {
    await dialogOrbit(
      ["export", path, "--format", selectedFormat, "--content", content],
      t("Exporting modpack", "正在导出整合包"),
      instance,
    );
    if (withRuntime)
      await dialogLauncher(
        ["export", path, "--base", path],
        t("Exporting worlds and runtime state", "正在导出世界及运行时状态"),
        instance,
      );
    exportOpen.value = false;
  } finally {
    exporting.value = false;
  }
}
async function importPack() {
  const instance = selected();
  const path = await open({
    multiple: false,
    filters: [
      {
        name: t("Modpack", "整合包"),
        extensions: ["orbitbundle", "mrpack", "toml"],
      },
    ],
  });
  if (typeof path !== "string") return;
  let optional: string[] = [];
  if (!path.endsWith(".toml")) {
    const pack = await launcher<{
      optional_files: { path: string; targets: string[] }[];
    }>(
      ["package", "inspect", path],
      t("Inspecting modpack", "正在检查整合包"),
      null,
    );
    optional = pack.optional_files
      .filter((f) => f.targets.includes(instance.kind))
      .map((f) => f.path);
  }
  if (workspace.selected?.id === instance.id)
    importReview.value = { path, optional, selected: [...optional] };
}
async function importConfirmed() {
  const pack = importReview.value;
  if (!pack || importing.value) return;
  importing.value = true;
  try {
    await dialogMutate(
      importArguments(pack.path, pack.selected),
      t("Importing modpack", "正在导入整合包"),
    );
    importReview.value = undefined;
  } finally {
    importing.value = false;
  }
}
async function resolution(clear = false) {
  await launcher(
    clear
      ? ["instance", "resolution", "clear"]
      : [
          "instance",
          "resolution",
          "set",
          String(width.value),
          String(height.value),
        ],
    t("Updating launch resolution", "正在更新启动分辨率"),
  );
  if (workspace.selected) await selectInstance(workspace.selected);
}
async function status() {
  serverStatus.value = await launcher(
    ["server", "status"],
    t("Checking server status", "正在检查服务端状态"),
  );
}
</script>
<template>
  <div v-if="workspace.selected" class="page">
    <RouterLink class="back-link" to="/"
      ><ArrowLeft :size="16" />{{ t("Library", "实例库") }}</RouterLink
    >
    <header class="page-heading">
      <div class="row">
        <button
          :title="t('Change instance icon', '更改实例图标')"
          :disabled="changingIcon"
          @click="act(() => changeIcon())"
        >
          <InstanceIcon
            :src="workspace.detail?.icon_path"
            :name="workspace.selected.name"
            :kind="workspace.selected.kind"
            :size="56"
          />
        </button>
        <div>
          <h1>{{ workspace.selected.name }}</h1>
          <p>
            {{
              workspace.detail?.installed?.minecraft ||
              workspace.detail?.desired.minecraft
            }}
            ·
            {{
              workspace.detail?.installed?.loader ||
              workspace.detail?.desired.loader
            }}
            {{ workspace.detail?.installed?.loader_version }} ·
            {{
              workspace.selected.kind === "server"
                ? t("Server", "服务端")
                : t("Client", "客户端")
            }}
          </p>
        </div>
      </div>
      <div class="row">
        <Button
          :title="t('Open instance folder', '打开实例目录')"
          @click="act(() => openPath(selected().directory))"
          ><FolderOpen /></Button
        ><Button type="colored" @click="act(() => launch(selected()))"
          ><Play />{{
            workspace.selected.kind === "server"
              ? t("Start server", "启动服务端")
              : t("Play", "开始游戏")
          }}</Button
        >
      </div>
    </header>
    <nav class="tabs section-tabs">
      <button
        v-for="item in [
          ['mods', t('Content', '模组内容')],
          ['runtime', t('Installation', '运行时')],
          ...(workspace.detail?.desired.loader === 'vanilla'
            ? []
            : [['audit', t('Compatibility', '兼容性')]]),
          ...(workspace.selected.kind === 'server'
            ? [['server', t('Server', '服务端')]]
            : []),
        ]"
        :key="item[0]"
        :class="{ active: tab === item[0] }"
        @click="router.push(instancePath(selected().id, item[0]))"
      >
        {{ item[1] }}
      </button>
    </nav>
    <template v-if="tab === 'mods'">
      <div
        v-if="workspace.detail?.desired.loader === 'vanilla'"
        class="card stack"
      >
        <h2>{{ t("Vanilla Minecraft", "原版 Minecraft") }}</h2>
        <p>
          {{
            t(
              "This instance has no mod loader and does not need an Orbit workspace.",
              "此实例没有模组加载器，无需 Orbit 工作区。",
            )
          }}
        </p>
        <Button @click="router.push(instancePath(selected().id, 'runtime'))">{{
          t("Manage runtime", "管理运行时")
        }}</Button>
      </div>
      <div v-else-if="workspace.inventoryError" class="card stack">
        <p class="error">{{ workspace.inventoryError }}</p>
        <Button
          :disabled="
            !workspace.detail || workspace.detail.desired.loader === 'vanilla'
          "
          @click="act(initializeMods)"
          >{{ t("Initialize Orbit workspace", "初始化 Orbit 工作区") }}</Button
        >
      </div>
      <template v-else>
        <div class="toolbar">
          <input
            v-model="query"
            class="search"
            :placeholder="t('Filter installed mods', '筛选已安装模组')"
          /><Button
            type="colored"
            @click="router.push(discoveryPath(selected().id))"
            ><PackagePlus />{{ t("Add content", "添加模组") }}</Button
          ><ActionMenu :label="t('More content actions', '更多内容操作')">
            <button @click="act(importPack)">
              <Download :size="18" />{{ t("Import modpack", "导入整合包") }}
            </button>
            <button @click="exportOpen = true">
              <Upload :size="18" />{{ t("Export modpack", "导出整合包") }}
            </button>
            <hr />
            <button @click="maintenanceOpen = true">
              <Settings2 :size="18" /><span
                >{{ t("Repair & reconcile", "修复与对账")
                }}<small>{{
                  t("Changed files outside Orbit?", "手动更改过模组文件？")
                }}</small></span
              >
            </button>
          </ActionMenu>
        </div>
        <div class="toolbar">
          <div class="tabs grow">
            <button
              :class="{ active: !showUpdates }"
              @click="showUpdates = false"
            >
              {{ t("Installed", "已安装") }}
              {{ workspace.packages.length }}</button
            ><button
              :class="{ active: showUpdates }"
              @click="showUpdates = true"
            >
              {{ t("Updates", "更新") }} {{ updates?.updates.length ?? "—" }}
            </button>
          </div>
          <Button size="sm" :disabled="checking" @click="act(checkUpdates)"
            ><RefreshCw />{{ t("Check for updates", "检查更新") }}</Button
          ><Button
            v-if="updates?.updates.length"
            type="colored"
            @click="
              act(() =>
                mutate(['upgrade'], t('Upgrading mods', '正在升级模组')),
              )
            "
            ><ArrowUpCircle />{{ t("Update all", "全部升级") }}</Button
          >
        </div>
        <div class="list">
          <div v-for="pkg in packages" :key="pkg.mod_id" class="item-row">
            <Avatar :src="pkg.icon_path" :alt="pkg.mod_id" :size="44" />
            <div class="grow">
              <div class="row wrap">
                <button class="text-link" @click="manager = pkg">
                  <strong>{{ pkg.mod_id }}</strong></button
                ><span class="badge">{{ pkg.version }}</span
                ><span v-if="!pkg.enabled" class="badge">{{
                  t("Disabled", "已禁用")
                }}</span>
              </div>
              <small>{{
                pkg.environment === "client"
                  ? t("Client", "客户端")
                  : pkg.environment === "server"
                    ? t("Server", "服务端")
                    : t("Client & server", "客户端与服务端")
              }}</small>
              <p v-if="updates?.updates.find((u) => u.mod_id === pkg.mod_id)">
                →
                {{
                  updates.updates.find((u) => u.mod_id === pkg.mod_id)
                    ?.new_version
                }}
              </p>
            </div>
            <Button
              v-if="updates?.updates.some((u) => u.mod_id === pkg.mod_id)"
              type="quiet"
              :title="t('Upgrade', '升级')"
              @click="
                act(() =>
                  mutate(
                    ['upgrade', pkg.mod_id],
                    t('Upgrading', '正在升级') + ' ' + pkg.mod_id,
                  ),
                )
              "
              ><ArrowUpCircle />{{ t("Update", "升级") }}</Button
            ><Toggle
              :model-value="pkg.enabled"
              :disabled="activating.has(pkg.mod_id)"
              :aria-label="t('Enable ', '启用 ') + pkg.mod_id"
              @update:model-value="act(() => togglePackage(pkg))"
            />
            <ActionMenu :label="t('Manage ', '管理 ') + pkg.mod_id">
              <button @click="manager = pkg">
                <Settings2 :size="18" />{{ t("Manage mod", "管理模组") }}
              </button>
              <button
                @click="
                  act(() =>
                    mutate(
                      ['remove', pkg.mod_id],
                      t('Removing', '正在移除') + ' ' + pkg.mod_id,
                    ),
                  )
                "
              >
                <Trash2 :size="18" />{{ t("Remove mod", "移除模组") }}
              </button>
            </ActionMenu>
          </div>
        </div>
        <p v-if="!packages.length" class="empty">
          {{ t("No matching content.", "没有符合筛选条件的模组。") }}
        </p>
        <template v-if="showUpdates && updates"
          ><details
            v-for="diagnostic in updates.diagnostics"
            :key="diagnostic.package + diagnostic.candidate_version"
            class="card"
          >
            <summary>
              {{ diagnostic.package }} · {{ diagnostic.candidate_version }}
            </summary>
            <p v-for="fact in diagnostic.facts" :key="fact">{{ fact }}</p>
          </details>
          <p v-for="warning in updates.warnings" :key="warning">
            {{ warning }}
          </p></template
        >
      </template>
    </template>
    <template v-if="tab === 'runtime'">
      <div class="card row">
        <div class="grow">
          <h3>{{ t("Launch reports", "启动报告") }}</h3>
          <p>
            {{
              launchReportNote ||
              t(
                "Reports and bounded process logs stay available after closing the GUI.",
                "关闭 GUI 后，仍可查看本次报告和有大小限制的进程日志。",
              )
            }}
          </p>
        </div>
        <Button @click="act(showLaunchReport)">{{
          t("View latest report", "查看最近报告")
        }}</Button>
      </div>
      <div class="card stack">
        <h2>{{ t("Game installation", "游戏安装") }}</h2>
        <div class="row wrap">
          <span class="badge"
            >Minecraft {{ workspace.detail?.installed?.minecraft ?? "—" }}</span
          ><span class="badge"
            >{{ workspace.detail?.installed?.loader }}
            {{ workspace.detail?.installed?.loader_version }}</span
          ><span class="badge"
            >Java {{ workspace.detail?.installed?.java?.major ?? "—" }}</span
          >
        </div>
        <p class="path">{{ workspace.selected.directory }}</p>
        <div class="toolbar">
          <Button
            @click="
              router.push(instancePath(selected().id, 'create') + '?migrate=1')
            "
            ><ArrowUpCircle />{{
              t("Migrate to a new instance", "迁移到新实例")
            }}</Button
          ><Button
            :disabled="workspace.detail?.desired.loader === 'vanilla'"
            @click="
              router.push(instancePath(selected().id, 'create') + '?loader=1')
            "
            >{{ t("Update loader", "更新加载器") }}</Button
          ><Button
            @click="
              act(async () => {
                await launcher(
                  ['install'],
                  t('Verifying runtime', '正在校验运行时'),
                );
                await selectInstance(selected());
              })
            "
            ><ShieldCheck />{{ t("Verify & repair", "校验与修复") }}</Button
          >
        </div>
      </div>
      <div class="card stack">
        <h2>{{ t("Java runtime", "Java 运行时") }}</h2>
        <JavaChooser
          :model-value="workspace.detail?.java_selection"
          inherit
          :disabled="configuringJava"
          @update:model-value="(value) => act(() => changeJava(value))"
        />
      </div>
      <div class="card row">
        <InstanceIcon
          :src="workspace.detail?.icon_path"
          :name="workspace.selected.name"
          :kind="workspace.selected.kind"
          :size="56"
        />
        <div class="grow">
          <h3>{{ t("Instance icon", "实例图标") }}</h3>
          <p>
            {{
              t(
                "A personal image for this instance.",
                "为这个实例选择一张图片。",
              )
            }}
          </p>
        </div>
        <Button :disabled="changingIcon" @click="act(() => changeIcon())">{{
          t("Choose image…", "选择图片…")
        }}</Button
        ><Button
          v-if="workspace.detail?.icon_path"
          :disabled="changingIcon"
          @click="act(() => changeIcon(true))"
          >{{ t("Reset", "重置") }}</Button
        >
      </div>
      <MemorySettings
        v-if="workspace.detail?.memory"
        :value="workspace.detail.memory"
        :pending="memorySaving"
        :capabilities="memorySettings?.capabilities"
        :loading="memoryLoading"
        @retry="act(loadMemory)"
        @save="(value) => act(() => saveMemory(value))"
        @reset="act(() => saveMemory())"
      />
      <p v-else class="error">
        {{
          t(
            "Update Orbit Launcher to manage instance memory.",
            "请更新 Orbit Launcher 后管理实例内存。",
          )
        }}
      </p>
      <div v-if="workspace.selected.kind === 'client'" class="card stack">
        <h2>{{ t("Window resolution", "窗口分辨率") }}</h2>
        <p>
          {{
            t(
              "Only affects the game client, not the server.",
              "仅影响游戏客户端，不用于服务端启动。",
            )
          }}
        </p>
        <div class="row">
          <input
            v-model.number="width"
            type="number"
            min="1"
            :aria-label="t('Width', '宽度')"
          /><span>×</span
          ><input
            v-model.number="height"
            type="number"
            min="1"
            :aria-label="t('Height', '高度')"
          /><Button @click="act(() => resolution())">{{
            t("Save", "保存")
          }}</Button
          ><Button type="quiet" @click="act(() => resolution(true))">{{
            t("Use game default", "使用游戏默认值")
          }}</Button>
        </div>
      </div>
      <div class="card row">
        <div class="grow">
          <h3>{{ t("Instance identity", "实例身份") }}</h3>
          <p>{{ workspace.selected.name }}</p>
        </div>
        <Button
          @click="
            newName = selected().name;
            renameOpen = true;
          "
          >{{ t("Rename", "重命名") }}</Button
        ><Button @click="unregister = true">{{
          t("Remove from library…", "从实例库移除…")
        }}</Button
        ><Button @click="act(reviewDeletion)">{{
          t("Delete instance…", "删除实例…")
        }}</Button>
      </div>
      <div class="card row">
        <div class="grow">
          <h3>{{ t("Default installation", "默认实例") }}</h3>
          <p>
            {{
              t(
                "Use this installation when no local context is available.",
                "没有局部上下文时使用此实例。",
              )
            }}
          </p>
        </div>
        <Button
          @click="
            act(() =>
              launcher(
                ['instance', 'default', 'set', selected().id],
                t('Setting default instance', '正在设置默认实例'),
                null,
              ),
            )
          "
          >{{ t("Set as default", "设为默认") }}</Button
        >
      </div>
    </template>
    <template v-if="tab === 'audit'">
      <div class="card stack">
        <h2>{{ t("Bytecode compatibility", "字节码兼容性") }}</h2>
        <p>
          {{
            t(
              "Static analysis reports risks, not a guarantee that the game will run.",
              "静态分析提供兼容风险信息，不保证游戏一定能够运行。",
            )
          }}
        </p>
        <div class="toolbar">
          <input
            v-model="auditFilter"
            :placeholder="t('All mods', '全部模组')"
          /><DropdownSelect
            v-model="minRisk"
            :label="t('Risk level', '风险等级')"
            :searchable="false"
            :options="[
              { value: '0', label: t('All risks', '全部风险') },
              { value: '35', label: t('Medium and above', '中等及以上') },
              { value: '70', label: t('High risk', '高风险') },
            ]"
          />
          <Button
            type="colored"
            :disabled="auditBusy"
            @click="act(() => runAudit())"
            ><ShieldCheck />{{ t("Run audit", "运行审计") }}</Button
          ><Button
            :disabled="auditBusy"
            @click="
              act(async () => {
                const path = await save({ defaultPath: 'orbit-audit.json' });
                if (path) await runAudit(path);
              })
            "
            >{{ t("Save detailed report", "保存详细报告") }}</Button
          >
        </div>
      </div>
      <template v-if="audit"
        ><div class="card">
          <h3>{{ audit.readiness_message }}</h3>
          <p>
            {{ audit.artifacts }}
            {{ t("artifacts inspected", "个工件已分析") }} ·
            {{ audit.runtime_namespace }}
          </p>
        </div>
        <div
          v-for="(finding, index) in audit.findings"
          :key="index"
          class="card stack"
        >
          <div class="row">
            <strong class="grow">{{ finding.packages }}</strong
            ><span class="badge"
              >{{ finding.risk }} · {{ finding.severity }}</span
            >
          </div>
          <p>{{ finding.reason }}</p>
          <small>{{ finding.rule }} · {{ finding.confidence }}</small>
        </div>
        <details
          v-if="audit.warnings.length || audit.coverage_gaps.length"
          class="card"
        >
          <summary>
            {{ t("Analysis coverage and notices", "分析覆盖情况与提示") }}
          </summary>
          <p
            v-for="(notice, index) in [
              ...audit.warnings,
              ...audit.coverage_gaps,
            ]"
            :key="index"
          >
            {{ notice.detail }}
          </p>
        </details></template
      >
    </template>
    <template v-if="tab === 'server' && workspace.selected.kind === 'server'">
      <div class="card stack">
        <h2>{{ t("Server controls", "服务端控制") }}</h2>
        <div class="toolbar">
          <span v-if="serverStatus" class="badge">{{
            serverStatus.running
              ? t("Running", "正在运行")
              : t("Stopped", "已停止")
          }}</span
          ><Button @click="act(status)">{{
            t("Refresh status", "刷新状态")
          }}</Button
          ><Button
            @click="
              act(async () => {
                await launcher(
                  ['server', 'stop'],
                  t('Stopping server', '正在停止服务端'),
                );
                await status();
              })
            "
            >{{ t("Stop server", "停止服务端") }}</Button
          ><Button
            @click="
              act(async () => {
                eula = await launcher(
                  ['server', 'eula', 'show'],
                  t('Loading Minecraft EULA', '正在加载 Minecraft EULA'),
                );
              })
            "
            >{{ t("Minecraft EULA", "Minecraft 最终用户许可协议") }}</Button
          >
        </div>
        <form
          class="row"
          @submit.prevent="
            act(async () => {
              await launcher(
                ['server', 'command', ...serverCommand.trim().split(/\s+/)],
                t('Sending server command', '正在发送服务端命令'),
              );
              serverCommand = '';
            })
          "
        >
          <input
            v-model="serverCommand"
            class="grow"
            :placeholder="t('Server console command', '服务端控制台命令')"
          /><Button
            :disabled="!serverCommand.trim()"
            @click="
              act(async () => {
                await launcher(
                  ['server', 'command', ...serverCommand.trim().split(/\s+/)],
                  t('Sending server command', '正在发送服务端命令'),
                );
                serverCommand = '';
              })
            "
            >{{ t("Send", "发送") }}</Button
          >
        </form>
      </div>
    </template>
    <Modal
      v-if="renameOpen"
      :task="dialogTask"
      :title="t('Rename instance', '重命名实例')"
      @close="renameOpen = false"
      ><label class="field"
        >{{ t("Name", "名称") }}<input v-model="newName" /></label
      ><template #footer
        ><Button
          type="colored"
          :disabled="!newName.trim()"
          @click="
            act(async () => {
              const instance = selected();
              await dialogLauncher(
                ['instance', 'rename', newName],
                t('Renaming instance', '正在重命名实例'),
                instance,
              );
              renameOpen = false;
              await refresh();
            })
          "
          >{{ t("Save", "保存") }}</Button
        ></template
      ></Modal
    >
    <Modal
      v-if="deleteOpen && deletion"
      :title="t('Delete the entire instance?', '删除整个实例？')"
      :task="dialogTask"
      :closable="!deleting"
      @close="deleteOpen = false"
    >
      <div class="stack">
        <strong>{{ deletion.name }}</strong>
        <p style="overflow-wrap: anywhere">{{ deletion.directory }}</p>
        <p>
          {{
            t(
              "This deletes the complete instance directory, including worlds, mods, settings and screenshots. Shared libraries and other instances are not deleted.",
              "这会删除整个实例目录，包括世界、模组、设置与截图。共享运行库和其他实例不会删除。",
            )
          }}
        </p>
        <p>
          {{
            t(
              "The directory is moved to the system trash. If trash is unavailable, deletion fails without falling back to permanent removal. Close the game first.",
              "目录将移入系统回收站；若回收站不可用则报错，不会改为永久删除。请先关闭游戏。",
            )
          }}
        </p>
      </div>
      <template #footer
        ><Button :disabled="deleting" @click="deleteOpen = false">{{
          t("Cancel", "取消")
        }}</Button
        ><Button
          type="colored"
          :loading="deleting"
          @click="act(deleteInstance)"
          >{{ t("Delete entire instance", "删除整个实例") }}</Button
        ></template
      >
    </Modal>
    <Modal
      v-if="unregister"
      :task="dialogTask"
      :title="t('Remove from library?', '从实例库移除？')"
      @close="unregister = false"
      ><p>
        {{
          t(
            "Only unregisters this instance. Its game files, mods and worlds are kept on disk.",
            "仅移除实例注册记录。游戏文件、模组和世界都保留在磁盘上。",
          )
        }}
      </p>
      <template #footer
        ><Button @click="unregister = false">{{ t("Cancel", "取消") }}</Button
        ><Button
          type="colored"
          @click="
            act(async () => {
              await dialogLauncher(
                ['instance', 'remove'],
                t('Unregistering instance', '正在取消实例注册'),
                selected(),
              );
              unregister = false;
              await refresh();
              await router.push('/');
            })
          "
          >{{ t("Remove from library", "从实例库移除") }}</Button
        ></template
      ></Modal
    >
    <Modal
      v-if="maintenanceOpen"
      :title="t('What needs attention?', '需要处理什么？')"
      @close="maintenanceOpen = false"
    >
      <div class="maintenance-options">
        <button
          @click="
            maintenanceOpen = false;
            act(() =>
              mutate(['sync'], t('Synchronizing mods', '正在同步模组')),
            );
          "
        >
          <RefreshCw /><span
            ><strong>{{
              t("I changed files in the mods folder", "我手动改了模组文件")
            }}</strong
            ><small>{{
              t(
                "Record what is on disk. Does not solve dependencies or delete mods.",
                "重新记录磁盘现状，不求解依赖，也不删除模组。",
              )
            }}</small></span
          >
        </button>
        <button
          @click="
            maintenanceOpen = false;
            act(() =>
              mutate(['fix'], t('Fixing dependencies', '正在修复依赖')),
            );
          "
        >
          <ShieldCheck /><span
            ><strong>{{
              t("Resolve dependency conflicts", "解决依赖冲突")
            }}</strong
            ><small>{{
              t(
                "Find minimal-change plans and review them before applying.",
                "查找改动尽量少的方案，由你选择并确认后应用。",
              )
            }}</small></span
          >
        </button>
        <button
          @click="
            maintenanceOpen = false;
            act(() =>
              mutate(
                ['install'],
                t('Restoring locked files', '正在恢复锁定文件'),
              ),
            );
          "
        >
          <Download /><span
            ><strong>{{
              t("Restore missing package files", "恢复缺失的模组文件")
            }}</strong
            ><small>{{
              t(
                "Install the exact versions already recorded in the lockfile.",
                "仅安装锁文件中已记录的精确版本，不重新选版本。",
              )
            }}</small></span
          >
        </button>
      </div>
      <template #footer
        ><Button @click="maintenanceOpen = false">{{
          t("Cancel", "取消")
        }}</Button></template
      >
    </Modal>
    <PackageManager
      v-if="manager"
      :key="manager.mod_id"
      :pkg="manager"
      @close="manager = undefined"
    />
    <Modal
      v-if="exportOpen"
      :task="dialogTask"
      :closable="!exporting"
      :title="t('Export modpack', '导出整合包')"
      @close="exportOpen = false"
      ><div class="stack">
        <label class="field"
          >{{ t("Format", "格式")
          }}<DropdownSelect
            v-model="format"
            :disabled="exporting"
            :label="t('Format', '格式')"
            :searchable="false"
            :options="[
              { value: 'orbit', label: 'Orbit (.orbitbundle)' },
              { value: 'mrpack', label: 'Modrinth (.mrpack)' },
            ]" /></label
        ><label
          ><input v-model="includeData" type="checkbox" />{{
            t("Include owned mod data", "包含模组所属数据")
          }}</label
        ><label v-if="format === 'orbit'"
          ><input v-model="includeRuntime" type="checkbox" />{{
            t("Include worlds and Launcher state", "包含世界与 Launcher 状态")
          }}</label
        >
      </div>
      <template #footer
        ><Button :disabled="exporting" @click="exportOpen = false">{{
          t("Close", "关闭")
        }}</Button
        ><Button type="colored" :loading="exporting" @click="act(exportPack)">{{
          t("Choose destination…", "选择导出位置…")
        }}</Button></template
      ></Modal
    >
    <Modal
      v-if="importReview"
      :task="dialogTask"
      :closable="!importing"
      :title="t('Import modpack', '导入整合包')"
      @close="importReview = undefined"
      ><div class="stack">
        <p>{{ importReview.path }}</p>
        <p>
          {{
            t(
              "Package files and configuration will be merged into this instance. No dependency repair is performed.",
              "整合包文件及配置将合并到当前实例，不自动求解或修复依赖。",
            )
          }}
        </p>
        <label v-for="file in importReview.optional" :key="file"
          ><input
            v-model="importReview.selected"
            type="checkbox"
            :value="file"
          />{{ file }}</label
        >
      </div>
      <template #footer
        ><Button :disabled="importing" @click="importReview = undefined">{{
          t("Cancel", "取消")
        }}</Button
        ><Button
          type="colored"
          :loading="importing"
          @click="act(importConfirmed)"
          >{{ t("Import", "导入") }}</Button
        ></template
      ></Modal
    >
    <Modal v-if="eula" title="Minecraft EULA" wide @close="eula = undefined"
      ><p style="white-space: pre-wrap">{{ eula.text }}</p>
      <template #footer
        ><Button @click="eula = undefined">{{ t("Decline", "不同意") }}</Button
        ><Button
          type="colored"
          @click="
            act(async () => {
              await launcher(
                ['server', 'eula', 'accept', eula!.digest_sha256],
                t('Accepting EULA', '正在接受 EULA'),
              );
              eula = undefined;
            })
          "
          >{{ t("I agree", "我同意") }}</Button
        ></template
      ></Modal
    >
  </div>
  <div v-else class="page empty">
    <h2>
      {{ t("Choose an instance from your library", "请从实例库选择实例") }}
    </h2>
    <Button @click="router.push('/')">{{ t("Library", "实例库") }}</Button>
  </div>
</template>
<style scoped>
.maintenance-options {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.maintenance-options button {
  display: flex;
  align-items: center;
  gap: 16px;
  text-align: left;
  padding: 18px;
  border-radius: 12px;
  background: var(--surface-3);
}
.maintenance-options button:hover {
  background: var(--surface-4);
}
.maintenance-options span {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.path {
  overflow-wrap: anywhere;
}
.item-row small {
  overflow-wrap: anywhere;
}
details > p {
  margin-top: 12px;
}
</style>
