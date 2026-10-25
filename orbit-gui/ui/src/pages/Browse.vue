<script setup lang="ts">
import { computed, ref, onMounted, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import {
  Search,
  Plus,
  Download,
  Check,
  ArrowLeft,
  LoaderCircle,
  SlidersHorizontal,
} from "lucide-vue-next";
import { workspace, orbit, mutate, act } from "../services/workspace";
import { Cancelled } from "../services/cli";
import {
  discoverySession,
  projectKey,
  installedPackages,
  addArguments,
  type DiscoveryState,
} from "../services/discovery";
import { instancePath } from "../services/navigation";
import { t } from "../i18n";
import type { SearchResult, Package, Instance } from "../types";
import Button from "../components/upstream/Button.vue";
import Avatar from "../components/Avatar.vue";
import Modal from "../components/Modal.vue";
import PackageManager from "../components/PackageManager.vue";
const route = useRoute(),
  router = useRouter();
const instance = computed(() =>
  workspace.instances.find((i) => i.id === route.params.id),
);
const state = computed(() => discoverySession(instance.value?.id));
const chooseTarget = ref<SearchResult>();
const manager = ref<Package>();
watch(
  () => instance.value && workspace.detail?.desired,
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
  const revision = ++session.revision;
  session.loading = true;
  session.error = "";
  try {
    const runtime = target ? workspace.detail?.desired : undefined;
    const data = await orbit<{ results: SearchResult[] }>(
      [
        "search",
        session.query,
        ...(runtime ? ["--mc-version", runtime.minecraft] : []),
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
async function install(
  project: SearchResult,
  target = instance.value,
  options: Pick<
    DiscoveryState,
    "recommended" | "environment" | "optional"
  > = state.value,
) {
  if (!target) {
    chooseTarget.value = project;
    return;
  }
  const session = discoverySession(target.id);
  if (session.installing) return;
  const args = addArguments(project, options);
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
async function targetSelected(target: Instance) {
  const project = chooseTarget.value;
  if (!project) return;
  const { recommended, environment, optional } = state.value;
  const options = { recommended, environment, optional };
  Object.assign(discoverySession(target.id), options);
  chooseTarget.value = undefined;
  await router.push({
    path: instancePath(target.id, "browse"),
    query: { q: project.name },
  });
  await install(project, target, options);
}
async function manage(project: SearchResult) {
  const packages = installed(project);
  if (packages.length === 1) manager.value = packages[0];
  else if (instance.value) await router.push(instancePath(instance.value.id));
}
onMounted(() => {
  if (typeof route.query.q === "string") {
    state.value.query = route.query.q;
    void search();
  }
});
</script>
<template>
  <div class="page discover-page">
    <RouterLink
      v-if="instance"
      class="back-link"
      :to="instancePath(instance.id)"
      ><ArrowLeft :size="16" />{{ instance.name }} /
      {{ t("Content", "模组内容") }}</RouterLink
    >
    <header class="page-heading">
      <div>
        <h1>
          {{
            instance
              ? t("Add content", "添加模组")
              : t("Discover mods", "发现模组")
          }}
        </h1>
        <p v-if="instance">
          {{ t("Installing to", "安装到") }}
          <strong>{{ instance.name }}</strong> ·
          {{ workspace.detail?.desired.minecraft }} ·
          {{ workspace.detail?.desired.loader }}
        </p>
        <p v-else>
          {{
            t(
              "Find a mod, then choose where to install it.",
              "发现模组，然后选择安装到哪个实例。",
            )
          }}
        </p>
      </div>
      <Button v-if="instance" @click="router.push(instancePath(instance.id))"
        ><Check />{{ t("Done", "返回已安装") }}</Button
      >
    </header>
    <div class="discover-layout">
      <aside class="discover-options stack">
        <h3>
          <SlidersHorizontal :size="16" />{{
            t("Install preferences", "添加选项")
          }}
        </h3>
        <label
          ><input v-model="state.recommended" type="checkbox" />{{
            t("Exclude beta / snapshot", "排除 beta / snapshot")
          }}</label
        >
        <small>{{
          t(
            "Only new packages use this editable string rule. Existing rules stay unchanged.",
            "仅为新包设置此可编辑的字符串规则，不更改已有包。",
          )
        }}</small>
        <hr />
        <label v-if="instance"
          ><input v-model="state.hideInstalled" type="checkbox" />{{
            t("Hide installed", "隐藏已安装")
          }}</label
        >
        <details>
          <summary>{{ t("Advanced options", "高级添加选项") }}</summary>
          <div class="stack">
            <label class="field"
              >{{ t("Environment", "运行环境")
              }}<select v-model="state.environment">
                <option value="auto">
                  {{ t("Follow JAR declaration", "遵循 JAR 声明") }}
                </option>
                <option value="client">{{ t("Client", "客户端") }}</option>
                <option value="server">{{ t("Server", "服务端") }}</option>
                <option value="both">{{ t("Both", "两端") }}</option>
              </select></label
            >
            <label
              ><input v-model="state.optional" type="checkbox" />{{
                t("Optional package", "可选包")
              }}</label
            >
          </div>
        </details>
      </aside>
      <section class="stack results-column">
        <form class="toolbar search-bar" @submit.prevent="search">
          <Search :size="20" /><input
            v-model="state.query"
            class="search"
            :aria-label="t('Search mods', '搜索模组')"
            :placeholder="t('Search mods…', '搜索模组…')"
          /><Button
            native-type="submit"
            type="colored"
            :loading="state.loading"
            :disabled="!state.query.trim()"
            >{{ t("Search", "搜索") }}</Button
          >
        </form>
        <p v-if="state.error" role="alert" class="error card">
          {{ state.error }}
        </p>
        <small v-if="state.searched"
          >{{ results.length }} {{ t("results", "个结果") }}</small
        >
        <article
          v-for="result in results"
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
        <div v-if="!results.length" class="empty">
          <Search :size="32" />
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
              instance
                ? t(
                    "Results are filtered for this Minecraft version and loader.",
                    "按此实例的 Minecraft 版本和加载器筛选结果。",
                  )
                : t(
                    "Search Modrinth and your configured CurseForge catalog.",
                    "搜索 Modrinth 与已配置的 CurseForge 项目。",
                  )
            }}
          </p>
        </div>
      </section>
    </div>
    <Modal
      v-if="chooseTarget"
      :title="t('Choose an instance', '选择实例')"
      @close="chooseTarget = undefined"
    >
      <p>
        {{ chooseTarget.name }} ·
        {{
          t(
            "Choose where to install. Orbit will check compatible versions before applying changes.",
            "选择安装位置。Orbit 会先检查兼容版本，再确认应用更改。",
          )
        }}
      </p>
      <div class="stack target-list">
        <Button
          v-for="target in workspace.instances"
          :key="target.id"
          @click="targetSelected(target)"
          >{{ target.name }} ·
          {{
            target.kind === "server"
              ? t("Server", "服务端")
              : t("Client", "客户端")
          }}</Button
        >
      </div>
      <template #footer
        ><Button @click="chooseTarget = undefined">{{
          t("Cancel", "取消")
        }}</Button
        ><Button @click="router.push('/create')">{{
          t("Create instance", "创建实例")
        }}</Button></template
      >
    </Modal>
    <PackageManager
      v-if="manager"
      :pkg="manager"
      @close="manager = undefined"
    />
  </div>
</template>
<style scoped>
.discover-layout {
  display: grid;
  grid-template-columns: 220px minmax(0, 1fr);
  gap: 24px;
  align-items: start;
}
.discover-options {
  position: sticky;
  top: 20px;
  padding: 16px;
  border-radius: 12px;
  background: var(--surface-2);
  gap: 16px;
}
.discover-options h3 {
  display: flex;
  align-items: center;
  gap: 8px;
}
.discover-options small {
  line-height: 1.6;
}
.discover-options details .stack {
  margin-top: 14px;
}
.discover-options hr {
  border-color: var(--surface-5);
}
.search-bar {
  flex-wrap: nowrap;
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
.target-list {
  margin-top: 16px;
}
@media (max-width: 1100px) {
  .discover-layout {
    grid-template-columns: 190px minmax(0, 1fr);
    gap: 16px;
  }
  .project-actions {
    grid-column: 2 / -1;
    justify-self: end;
  }
}
</style>
