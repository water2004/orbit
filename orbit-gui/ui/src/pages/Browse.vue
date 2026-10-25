<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import {
  Search,
  Plus,
  Download,
  Check,
  LoaderCircle,
  SlidersHorizontal,
} from "lucide-vue-next";
import {
  workspace,
  orbit,
  mutate,
  act,
  selectInstance,
} from "../services/workspace";
import { Cancelled } from "../services/cli";
import {
  discoverySession,
  projectKey,
  installedPackages,
  addArguments,
} from "../services/discovery";
import { instancePath, discoveryPath } from "../services/navigation";
import { imageUrl } from "../services/cli";
import DropdownSelect from "../components/upstream/DropdownSelect.vue";
import ProgressSpinner from "../components/upstream/ProgressSpinner.vue";
import { t } from "../i18n";
import type { SearchResult, Package } from "../types";
import Button from "../components/upstream/Button.vue";
import Avatar from "../components/Avatar.vue";
import PackageManager from "../components/PackageManager.vue";
import CatalogSettings from "../components/CatalogSettings.vue";
const route = useRoute(),
  router = useRouter();
const targetId = computed(() =>
  typeof route.query.instance === "string" ? route.query.instance : "",
);
const instance = computed(() =>
  workspace.instances.find((i) => i.id === targetId.value),
);
const state = computed(() =>
  discoverySession(targetId.value || "__no_instance__"),
);
const selecting = ref(false);
const savingCatalogs = ref(new Set<string>());
const savingCurrentCatalogs = computed(() =>
  savingCatalogs.value.has(targetId.value),
);
function catalogBusy(id: string, busy: boolean) {
  if (busy) savingCatalogs.value.add(id);
  else savingCatalogs.value.delete(id);
}
const ready = computed(
  () =>
    !!instance.value &&
    !selecting.value &&
    workspace.selected?.id === instance.value.id &&
    !!workspace.detail,
);
const targetOptions = computed(() =>
  workspace.instances.map((i) => ({
    value: i.id,
    label: i.name,
    description: [
      i.game_version,
      i.loader,
      i.kind === "server" ? t("Server", "服务端") : t("Client", "客户端"),
    ]
      .filter(Boolean)
      .join(" · "),
    icon:
      imageUrl(i.icon_path || undefined) ||
      (i.kind === "server"
        ? "/upstream/terminal.png"
        : "/upstream/grass-block.png"),
  })),
);
let selection = 0;
const manager = ref<Package>();
watch(ready, (value) => {
  if (
    value &&
    state.value.query.trim() &&
    !state.value.searched &&
    !state.value.loading
  )
    void search();
});
watch(
  () => workspace.selected?.id,
  (id) => {
    if (!targetId.value && id) void router.replace(discoveryPath(id));
  },
  { immediate: true },
);
watch(
  () => [targetId.value, workspace.instances] as const,
  async ([id], previous) => {
    if (!id) {
      if (workspace.selected)
        await router.replace(discoveryPath(workspace.selected.id));
      return;
    }
    const target = instance.value;
    if (!target) return;
    const revision = ++selection;
    manager.value = undefined;
    const previousId = previous?.[0];
    const previousQuery = previousId ? discoverySession(previousId).query : "";
    if (previousId && previousId !== id && previousQuery) {
      state.value.query = previousQuery;
      state.value.results = [];
      state.value.searched = false;
    }
    if (typeof route.query.q === "string") state.value.query = route.query.q;
    selecting.value = true;
    try {
      if (workspace.selected?.id !== id) await selectInstance(target);
    } finally {
      if (revision === selection) selecting.value = false;
    }
    if (
      revision === selection &&
      ready.value &&
      state.value.query.trim() &&
      !state.value.searched
    )
      await search();
  },
  { immediate: true },
);
watch(
  () => ready.value && workspace.detail?.desired,
  (runtime) => {
    if (!runtime) return;
    const session = state.value;
    const scope = JSON.stringify([runtime.minecraft, runtime.loader]);
    if (session.scope && session.scope !== scope) {
      session.results = [];
      session.searched = false;
      session.loading = false;
      session.revision++;
    }
    session.scope = scope;
  },
  { immediate: true },
);
const installed = (p: SearchResult) =>
  instance.value ? installedPackages(p, workspace.packages) : [];
const results = computed(() =>
  state.value.results.filter(
    (p) => !state.value.hideInstalled || !installed(p).length,
  ),
);
async function search() {
  const session = state.value,
    target = instance.value;
  if (!target || selecting.value || savingCatalogs.value.has(target.id)) return;
  const runtime = workspace.detail?.desired;
  if (!runtime) {
    session.error = t(
      "The instance runtime is unavailable. Check its installation before searching.",
      "无法读取实例运行时，请先检查实例安装。",
    );
    return;
  }
  const revision = ++session.revision;
  session.loading = true;
  session.error = "";
  try {
    const data = await orbit<{ results: SearchResult[] }>(
      [
        "search",
        session.query,
        "--mc-version",
        runtime.minecraft,
        ...(runtime?.loader && runtime.loader !== "vanilla"
          ? ["--modloader", runtime.loader]
          : []),
      ],
      t("Searching mods", "正在搜索模组"),
      target ?? null,
    );
    if (revision === session.revision) {
      session.results = data.results;
      session.searched = true;
    }
  } catch (e) {
    if (revision === session.revision && !(e instanceof Cancelled))
      session.error = String(e);
  } finally {
    if (revision === session.revision) session.loading = false;
  }
}
async function catalogsChanged() {
  const session = state.value;
  session.revision++;
  session.results = [];
  session.searched = false;
  session.loading = false;
  // The settings command has finished; refresh the same query using CLI policy.
  savingCatalogs.value.delete(targetId.value);
  if (session.query.trim()) await search();
}
async function install(project: SearchResult) {
  const target = instance.value;
  if (!target) return;
  const session = discoverySession(target.id);
  if (session.installing) return;
  const args = addArguments(project, session);
  session.installing = projectKey(project);
  session.error = "";
  try {
    await mutate(args, t("Adding ", "正在添加 ") + project.name, target);
  } catch (e) {
    if (!(e instanceof Cancelled)) session.error = String(e);
  } finally {
    session.installing = null;
  }
}
async function manage(project: SearchResult) {
  const packages = installed(project);
  if (packages.length === 1) manager.value = packages[0];
  else if (instance.value) await router.push(instancePath(instance.value.id));
}
</script>
<template>
  <div class="page discover-page">
    <header class="page-heading">
      <div>
        <h1>{{ t("Discover mods", "发现模组") }}</h1>
        <p>
          {{
            t(
              "Choose an instance to browse compatible mods.",
              "选择实例，浏览与其兼容的模组。",
            )
          }}
        </p>
      </div>
    </header>
    <div v-if="!workspace.instances.length && !workspace.loading" class="empty">
      <h2>{{ t("Create your first instance", "先创建一个实例") }}</h2>
      <Button @click="router.push('/create')"
        ><Plus />{{ t("Create instance", "创建实例") }}</Button
      >
    </div>
    <p v-else-if="targetId && !instance" class="error">
      {{
        t(
          "This instance is not available. Select another instance.",
          "此实例不存在，请选择其他实例。",
        )
      }}
    </p>
    <div v-else-if="instance" class="discover-layout">
      <form class="search-bar" @submit.prevent="search">
        <DropdownSelect
          class="target-selector"
          :model-value="targetId"
          :options="targetOptions"
          :label="t('Target instance', '目标实例')"
          :loading="workspace.loading && !workspace.instances.length"
          @update:model-value="(id) => router.replace(discoveryPath(id))"
        >
          <template #selected="{ option }"
            ><img
              v-if="option?.icon"
              :src="option.icon"
              alt=""
              class="target-icon"
            /><span class="target-name">{{
              option?.label || t("Select instance", "选择实例")
            }}</span></template
          >
        </DropdownSelect>
        <Search :size="18" /><input
          v-model="state.query"
          class="search"
          :aria-label="t('Search mods', '搜索模组')"
          :placeholder="t('Search mods…', '搜索模组…')"
        />
        <Button
          native-type="submit"
          type="colored"
          :loading="state.loading"
          :disabled="!state.query.trim() || selecting || savingCurrentCatalogs"
          >{{ t("Search", "搜索") }}</Button
        >
      </form>
      <CatalogSettings
        :key="instance.id"
        :instance="instance"
        :ready="ready"
        @busy="catalogBusy"
        @changed="catalogsChanged"
      />
      <div class="discover-options">
        <label
          ><input v-model="state.recommended" type="checkbox" />{{
            t("Exclude beta / snapshot", "排除 beta / snapshot")
          }}</label
        >
        <label v-if="instance"
          ><input v-model="state.hideInstalled" type="checkbox" />{{
            t("Hide installed", "隐藏已安装")
          }}</label
        >
        <details>
          <summary>
            <SlidersHorizontal :size="14" />{{
              t("Advanced options", "更多添加选项")
            }}
          </summary>
          <div class="stack">
            <label class="field"
              >{{ t("Environment", "运行环境")
              }}<DropdownSelect
                v-model="state.environment"
                :label="t('Environment', '运行环境')"
                :searchable="false"
                :options="[
                  {
                    value: 'auto',
                    label: t('Follow JAR declaration', '遵循 JAR 声明'),
                  },
                  { value: 'client', label: t('Client', '客户端') },
                  { value: 'server', label: t('Server', '服务端') },
                  { value: 'both', label: t('Both', '两端') },
                ]"
            /></label>
            <label
              ><input v-model="state.optional" type="checkbox" />{{
                t("Optional package", "可选包")
              }}</label
            >
          </div>
        </details>
        <small class="add-hint">{{
          t("Applies to new packages only.", "约束仅用于新添加的模组。")
        }}</small>
      </div>
      <section class="stack results-column">
        <p v-if="state.error" role="alert" class="error card">
          {{ state.error }}
        </p>
        <p v-if="selecting" class="row">
          <ProgressSpinner />{{ t("Loading instance…", "正在加载实例…") }}
        </p>
        <small v-if="state.searched"
          >{{ results.length }} {{ t("results", "个结果") }}</small
        >
        <article
          v-for="result in ready ? results : []"
          :key="projectKey(result)"
          class="project-card card"
        >
          <Avatar
            :src="result.icon_url"
            :alt="result.name"
            :tint-by="result.project_id"
            size="64px"
          />
          <div class="project-copy stack">
            <div class="row wrap">
              <h2>{{ result.name }}</h2>
              <span v-if="installed(result).length" class="installed-label"
                ><Check :size="14" />{{ t("Installed", "已安装") }}</span
              >
            </div>
            <p class="description">{{ result.description }}</p>
            <div class="row wrap project-meta">
              <span>{{ result.platform }}</span
              ><span
                ><Download :size="13" />{{
                  Intl.NumberFormat(undefined, { notation: "compact" }).format(
                    result.downloads,
                  )
                }}</span
              ><span v-if="result.latest_version">{{
                result.latest_version
              }}</span>
            </div>
          </div>
          <div class="project-actions">
            <Button v-if="state.installing === projectKey(result)" disabled
              ><LoaderCircle class="spin" />{{
                t("Installing…", "安装中…")
              }}</Button
            >
            <Button
              v-else-if="installed(result).length"
              @click="act(() => manage(result))"
              >{{ t("Manage", "管理") }}</Button
            >
            <Button
              v-else
              :disabled="!!state.installing || result.compatible === false"
              @click="install(result)"
              ><Plus />{{
                result.compatible === false
                  ? t("Incompatible", "不兼容")
                  : t("Install", "安装")
              }}</Button
            >
          </div>
        </article>
        <div v-if="!results.length && !selecting" class="empty">
          <ProgressSpinner v-if="state.loading" :size="32" /><Search
            v-else
            :size="32"
          />
          <h2>
            {{
              state.loading
                ? t("Searching…", "搜索中…")
                : state.searched
                  ? t("No matching mods", "没有匹配的模组")
                  : t("What would you like to add?", "想添加什么模组？")
            }}
          </h2>
          <p>
            {{
              t(
                "Results are filtered for this Minecraft version and loader.",
                "按此实例的 Minecraft 版本和加载器筛选结果。",
              )
            }}
          </p>
        </div>
      </section>
    </div>
    <PackageManager
      v-if="manager"
      :pkg="manager"
      @close="manager = undefined"
    />
  </div>
</template>
<style scoped>
.discover-layout {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.discover-page {
  max-width: 1240px;
  margin-inline: auto;
}
.target-selector {
  width: 250px;
}
.target-icon {
  width: 22px;
  height: 22px;
  object-fit: contain;
}
.target-name {
  flex: 1;
  text-overflow: ellipsis;
  white-space: nowrap;
  overflow: hidden;
  font-weight: 600;
}
.discover-options {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 12px 20px;
  padding-bottom: 12px;
  border-bottom: 1px solid var(--surface-4);
  font-size: 13px;
}
.discover-options summary {
  display: flex;
  align-items: center;
  gap: 5px;
  cursor: pointer;
}
.discover-options details[open] {
  flex-basis: 100%;
  order: 2;
}
.discover-options details .stack {
  margin-top: 12px;
  flex-direction: row;
  align-items: end;
  gap: 20px;
}
.discover-options .field {
  width: 240px;
}
.add-hint {
  margin-left: auto;
}
.results-column .empty {
  padding: 36px 16px;
}
.results-column .empty h2 {
  font-size: 17px;
}
.search-bar {
  display: flex;
  align-items: center;
  gap: 10px;
}
.project-card {
  display: grid;
  grid-template-columns: 64px minmax(0, 1fr) auto;
  align-items: start;
  gap: 16px;
}
.project-copy {
  gap: 8px;
}
.project-card h2 {
  font-size: 17px;
}
.description {
  line-height: 1.55;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
.project-meta {
  font-size: 12px;
  gap: 14px;
}
.project-meta span,
.installed-label {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}
.installed-label {
  color: var(--color-green);
  font-size: 12px;
}
@media (max-width: 1100px) {
  .project-actions {
    grid-column: 2 / -1;
    justify-self: end;
  }
}
@media (max-width: 760px) {
  .search-bar {
    flex-wrap: wrap;
  }
  .target-selector {
    width: 100%;
  }
  .add-hint {
    flex-basis: 100%;
    margin-left: 0;
  }
}
</style>
