<script setup lang="ts">
import { ref, watch } from "vue";
import { Search, Plus, Download } from "lucide-vue-next";
import { workspace, orbit, mutate, act } from "../services/workspace";
import { t } from "../i18n";
import type { SearchResult } from "../types";
import Button from "../components/upstream/Button.vue";
import Avatar from "../components/Avatar.vue";
import Modal from "../components/Modal.vue";
const query = ref("");
const results = ref<SearchResult[]>([]);
const loading = ref(false);
const searched = ref(false);
const adding = ref<SearchResult>();
const recommended = ref(true);
const environment = ref("auto");
const optional = ref(false);
let revision = 0;
watch(
  () => workspace.selected?.id,
  () => {
    revision++;
    results.value = [];
    searched.value = false;
    loading.value = false;
    adding.value = undefined;
  },
);
async function search() {
  const request = ++revision;
  loading.value = true;
  try {
    const runtime = workspace.detail?.desired;
    const args = [
      "search",
      query.value,
      ...(runtime ? ["--mc-version", runtime.minecraft] : []),
      ...(runtime?.loader && runtime.loader !== "vanilla"
        ? ["--modloader", runtime.loader]
        : []),
    ];
    const data = await orbit<{ results: SearchResult[] }>(
      args,
      t("Searching mods", "正在搜索模组"),
    );
    if (request === revision) {
      results.value = data.results;
      searched.value = true;
    }
  } finally {
    if (request === revision) loading.value = false;
  }
}
async function add() {
  const project = adding.value;
  if (!project) return;
  const args = [
    "add",
    `${project.platform === "modrinth" ? "mr" : "cf"}:${project.project_id}`,
  ];
  if (recommended.value)
    args.push(
      "--string",
      'all; intersect not contains(i"beta"); intersect not contains(i"snapshot")',
    );
  if (environment.value !== "auto") args.push("--env", environment.value);
  if (optional.value) args.push("--optional");
  adding.value = undefined;
  await mutate(args, t("Adding ", "正在添加 ") + project.name);
}
</script>
<template>
  <div class="page">
    <div class="page-heading">
      <div>
        <h1>{{ t("Discover content", "发现内容") }}</h1>
        <p>
          {{
            t(
              "Find something new for your installation.",
              "为当前实例发现更多玩法。",
            )
          }}
        </p>
      </div>
    </div>
    <form class="toolbar" @submit.prevent="act(search)">
      <Search :size="20" /><input
        v-model="query"
        class="search"
        :placeholder="t('Search mods…', '搜索模组…')"
      /><Button
        native-type="submit"
        type="colored"
        :loading="loading"
        :disabled="!query.trim()"
        >{{ t("Search", "搜索") }}</Button
      >
    </form>
    <div v-if="!workspace.selected" class="card">
      {{
        t(
          "Choose an installation before adding mods.",
          "添加模组前请先选择一个实例。",
        )
      }}
    </div>
    <div class="stack">
      <article
        v-for="result in results"
        :key="`${result.platform}:${result.project_id}`"
        class="project-card card"
      >
        <Avatar
          :src="result.icon_url"
          :alt="result.name"
          :tint-by="result.project_id"
          size="64px"
        />
        <div class="grow stack">
          <div class="row wrap">
            <h2>{{ result.name }}</h2>
            <span class="badge">{{ result.platform }}</span
            ><span v-if="result.compatible === false" class="error">{{
              t("Not compatible", "不兼容")
            }}</span>
          </div>
          <p>{{ result.description }}</p>
          <div class="row wrap">
            <span class="badge"
              ><Download :size="12" />{{
                Intl.NumberFormat(undefined, { notation: "compact" }).format(
                  result.downloads,
                )
              }}</span
            ><span class="badge">{{ result.latest_version }}</span
            ><span
              v-for="tag in result.categories.slice(0, 3)"
              :key="tag"
              class="badge"
              >{{ tag }}</span
            >
          </div>
        </div>
        <Button
          type="colored"
          :disabled="!workspace.selected"
          @click="
            adding = result;
            recommended = true;
            optional = false;
            environment = 'auto';
          "
          ><Plus />{{ t("Add", "添加") }}</Button
        >
      </article>
    </div>
    <div v-if="!results.length" class="empty">
      <Search :size="40" />
      <h2>
        {{
          loading
            ? t("Searching…", "搜索中…")
            : searched
              ? t("No results", "没有找到结果")
              : t("Make it your Minecraft", "打造你的 Minecraft")
        }}
      </h2>
      <p>
        {{
          t(
            "Search Modrinth and your configured CurseForge catalog.",
            "搜索 Modrinth 与已配置的 CurseForge 项目。",
          )
        }}
      </p>
    </div>
    <Modal
      v-if="adding"
      :title="t('Add ', '添加 ') + adding.name"
      @close="adding = undefined"
      ><div class="stack">
        <label
          ><input v-model="recommended" type="checkbox" />{{
            t(
              "Exclude version text containing beta or snapshot",
              "排除版本文本包含 beta 或 snapshot 的构建",
            )
          }}</label
        ><small>{{
          t(
            "This is an editable rule applied only to this new package.",
            "仅对这个新包添加可编辑的约束规则。",
          )
        }}</small
        ><label class="field"
          >{{ t("Environment", "环境")
          }}<select v-model="environment">
            <option value="auto">
              {{ t("Follow JAR declaration", "遵循 JAR 声明") }}
            </option>
            <option value="client">{{ t("Client", "客户端") }}</option>
            <option value="server">{{ t("Server", "服务端") }}</option>
            <option value="both">{{ t("Both", "两端") }}</option>
          </select></label
        ><label
          ><input v-model="optional" type="checkbox" />{{
            t("Optional package", "可选包")
          }}</label
        >
      </div>
      <template #footer
        ><Button @click="adding = undefined">{{ t("Cancel", "取消") }}</Button
        ><Button type="colored" @click="act(add)">{{
          t("Find installation plans", "查找安装方案")
        }}</Button></template
      ></Modal
    >
  </div>
</template>
<style scoped>
.project-card {
  display: flex;
  align-items: flex-start;
  gap: 18px;
}
.project-card p {
  line-height: 1.6;
}
.project-card h2 {
  font-size: 18px;
}
</style>
