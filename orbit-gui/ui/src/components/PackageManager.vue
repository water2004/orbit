<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { Folder, File, Plus, Trash2, LoaderCircle } from "lucide-vue-next";
import {
  act,
  mutate as mutateWorkspace,
  orbit,
  workspace,
} from "../services/workspace";
import { policyArguments } from "../services/commands";
import type {
  Ownership,
  Package,
  PackageVersions,
  StringRule,
  VersionPolicy,
} from "../types";
import { t } from "../i18n";
import Modal from "./Modal.vue";
import Button from "./upstream/Button.vue";
import StringRules from "./StringRules.vue";
const props = defineProps<{ pkg: Package }>();
const emit = defineEmits<{ close: [] }>();
const tab = ref("versions"),
  versions = ref<PackageVersions>(),
  ownership = ref<Ownership>(),
  error = ref(""),
  loading = ref(true),
  filesLoading = ref(false),
  saving = ref(false);
const policy = ref<VersionPolicy>({ kind: "any" }),
  rules = ref<StringRule>({ initial_all: true, operations: [] });
const mode = ref("any"),
  operator = ref("="),
  lower = ref(""),
  upper = ref(""),
  includeLower = ref(true),
  includeUpper = ref(true);
const provider = ref("modrinth"),
  locator = ref("");
const current = computed(
  () =>
    workspace.packages.find((p) => p.mod_id === props.pkg.mod_id) ?? props.pkg,
);
const environment = ref(props.pkg.configured_environment ?? "auto");
const instance = workspace.selected;
const mutate = (args: string[], label: string) =>
  mutateWorkspace(args, label, instance);
async function load() {
  loading.value = true;
  error.value = "";
  try {
    versions.value = await orbit<PackageVersions>(
      ["versions", props.pkg.mod_id],
      t("Loading available versions", "正在加载可用版本"),
      instance,
    );
    policy.value = versions.value.policy;
    rules.value = await invoke("parse_string_rule", {
      source: versions.value.string,
    });
    const p = policy.value;
    mode.value = p.kind;
    if (p.kind === "comparison") {
      operator.value = p.operator;
      lower.value = p.version;
    }
    if (p.kind === "range") {
      lower.value = p.lower;
      upper.value = p.upper;
      includeLower.value = p.include_lower;
      includeUpper.value = p.include_upper;
    }
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}
async function files() {
  if (filesLoading.value) return;
  tab.value = "files";
  filesLoading.value = true;
  error.value = "";
  try {
    ownership.value = await orbit<Ownership>(
      ["ownership", props.pkg.mod_id],
      t("Loading files and data", "正在加载文件与数据"),
      instance,
    );
  } catch (e) {
    error.value = String(e);
  } finally {
    filesLoading.value = false;
  }
}
async function apply() {
  saving.value = true;
  try {
    const next: VersionPolicy =
      mode.value === "any"
        ? { kind: "any" }
        : mode.value === "range"
          ? {
              kind: "range",
              lower: lower.value,
              upper: upper.value,
              include_lower: includeLower.value,
              include_upper: includeUpper.value,
            }
          : mode.value === "comparison"
            ? {
                kind: "comparison",
                operator: operator.value,
                version: lower.value,
              }
            : policy.value;
    const expression = await invoke<string>("render_string_rule", {
      draft: JSON.parse(JSON.stringify(rules.value)),
    });
    await mutate(
      [
        "constraint",
        "set",
        props.pkg.mod_id,
        ...policyArguments(next),
        "--string",
        expression,
      ],
      t("Applying version policy", "正在应用版本策略"),
    );
    await load();
  } finally {
    saving.value = false;
  }
}
onMounted(load);
</script>
<template>
  <Modal
    :title="`${pkg.mod_id} · ${current.version}`"
    wide
    @close="emit('close')"
  >
    <div class="stack">
      <nav class="tabs">
        <button
          :class="{ active: tab === 'versions' }"
          @click="tab = 'versions'"
        >
          {{ t("Versions", "数字版本") }}</button
        ><button
          :class="{ active: tab === 'strings' }"
          @click="tab = 'strings'"
        >
          {{ t("String filters", "字符串过滤") }}</button
        ><button :class="{ active: tab === 'files' }" @click="files">
          {{ t("Files & data", "文件与数据") }}</button
        ><button
          :class="{ active: tab === 'settings' }"
          @click="tab = 'settings'"
        >
          {{ t("Settings", "包设置") }}
        </button>
      </nav>
      <p v-if="error" class="error">{{ error }}</p>
      <p v-if="tab === 'files' ? filesLoading : loading" class="row">
        <LoaderCircle class="spin" />{{ t("Loading…", "正在加载…") }}
      </p>
      <template v-if="tab === 'versions' && versions">
        <div class="toolbar">
          <select v-model="mode">
            <option value="any">
              {{ t("Any compatible version", "任意兼容版本") }}
            </option>
            <option value="comparison">
              {{ t("One boundary", "单边界") }}
            </option>
            <option value="range">{{ t("Version range", "版本区间") }}</option>
            <option v-if="policy.kind === 'custom'" value="custom">
              {{
                t(
                  "Existing custom rule (unchanged)",
                  "现有自定义规则（未修改）",
                )
              }}
            </option>
          </select>
          <template v-if="mode === 'comparison'"
            ><select v-model="operator">
              <option v-for="op in ['=', '>', '>=', '<', '<=']" :key="op">
                {{ op }}
              </option></select
            ><select v-model="lower">
              <option
                v-for="version in [
                  ...new Set(versions.candidates.map((c) => c.version)),
                ]"
                :key="version"
              >
                {{ version }}
              </option>
            </select></template
          >
          <template v-if="mode === 'range'"
            ><select v-model="lower">
              <option
                v-for="version in [
                  ...new Set(versions.candidates.map((c) => c.version)),
                ]"
                :key="version"
              >
                {{ version }}
              </option></select
            ><label
              ><input v-model="includeLower" type="checkbox" />{{
                t("Include lower", "包含下界")
              }}</label
            ><span>—</span
            ><select v-model="upper">
              <option
                v-for="version in [
                  ...new Set(versions.candidates.map((c) => c.version)),
                ]"
                :key="version"
              >
                {{ version }}
              </option></select
            ><label
              ><input v-model="includeUpper" type="checkbox" />{{
                t("Include upper", "包含上界")
              }}</label
            ></template
          >
        </div>
        <p>
          {{
            t(
              "Choose a version below, then a boundary. Applying a policy solves the complete mod environment before changing anything.",
              "选择下方版本作为边界。应用策略前会对完整模组环境求解并让你确认方案。",
            )
          }}
        </p>
        <div class="list">
          <div
            v-for="(candidate, index) in versions.candidates"
            :key="index"
            class="item-row"
          >
            <div class="grow stack">
              <div class="row wrap">
                <strong>{{ candidate.version }}</strong
                ><span v-if="candidate.selected" class="badge">{{
                  t("Installed", "已安装")
                }}</span
                ><span v-if="!candidate.matches_constraint" class="badge">{{
                  t("Filtered", "已被策略过滤")
                }}</span>
              </div>
              <small>{{ candidate.details }}</small
              ><small v-if="!candidate.numeric_filterable">{{
                t(
                  "No numeric comparison; string filters still apply.",
                  "无法进行数字比较，字符串过滤仍然生效。",
                )
              }}</small>
            </div>
            <Button
              size="sm"
              @click="
                lower = candidate.version;
                mode = mode === 'range' ? 'range' : 'comparison';
              "
              >{{
                mode === "range" ? t("Lower", "设为下界") : t("Select", "选择")
              }}</Button
            ><Button
              v-if="mode === 'range'"
              size="sm"
              @click="upper = candidate.version"
              >{{ t("Upper", "设为上界") }}</Button
            >
          </div>
        </div>
      </template>
      <StringRules v-if="tab === 'strings' && versions" v-model="rules" />
      <template v-if="tab === 'files' && ownership">
        <h3>{{ t("Package files", "模组本体") }}</h3>
        <div
          v-for="file in ownership.artifacts"
          :key="file.path"
          class="row card"
        >
          <File /><span class="grow">{{ file.path }}</span
          ><span class="badge">{{
            file.present ? t("Present", "存在") : t("Missing", "缺失")
          }}</span>
        </div>
        <h3>{{ t("Owned data", "所属数据") }}</h3>
        <p v-if="!ownership.data.length">
          {{ t("No observed owned data.", "尚无已观测的所属数据。") }}
        </p>
        <div
          v-for="entry in ownership.data"
          :key="entry.path"
          class="card stack"
        >
          <div class="row">
            <Folder v-if="entry.kind === 'tree'" /><File v-else /><strong
              >{{ entry.path }}{{ entry.kind === "tree" ? "/**" : "" }}</strong
            ><span class="badge">{{ entry.scope }}</span>
          </div>
          <small v-for="keep in entry.preserved" :key="keep.path"
            >{{ t("Preserve", "保留") }} · {{ keep.path }}</small
          >
        </div>
        <div class="card danger-zone stack">
          <h3>{{ t("Reset mod data", "重置模组数据") }}</h3>
          <p>
            {{
              t(
                "Keeps the mod installed. Orbit will ask you to review the exact files before deleting them.",
                "保留模组本体。Orbit 会在删除前列出具体文件并要求确认。",
              )
            }}
          </p>
          <Button
            @click="
              act(async () => {
                await mutate(
                  ['reset', pkg.mod_id],
                  t('Resetting mod data', '正在重置模组数据'),
                );
                await files();
              })
            "
            >{{ t("Reset data…", "重置数据…") }}</Button
          >
        </div>
      </template>
      <template v-if="tab === 'settings'">
        <div class="card stack">
          <h3>{{ t("Environment", "运行环境") }}</h3>
          <div class="row">
            <select v-model="environment">
              <option value="auto">
                {{ t("Use JAR declaration", "按 JAR 声明") }}
              </option>
              <option value="client">{{ t("Client", "客户端") }}</option>
              <option value="server">{{ t("Server", "服务端") }}</option>
              <option value="both">{{ t("Both", "两端") }}</option></select
            ><Button
              @click="
                act(() =>
                  mutate(
                    ['env', pkg.mod_id, environment],
                    t('Updating environment', '正在更新环境'),
                  ),
                )
              "
              >{{ t("Apply", "应用") }}</Button
            >
          </div>
        </div>
        <div class="card stack">
          <h3>{{ t("Sources", "远端来源") }}</h3>
          <div
            v-for="(remote, index) in current.remotes"
            :key="remote"
            class="row"
          >
            <span class="grow">{{ remote }}</span
            ><Button
              type="quiet"
              :disabled="current.remotes.length === 1"
              :aria-label="t('Remove source', '移除来源')"
              @click="
                act(() =>
                  mutate(
                    ['remote', 'remove', pkg.mod_id, '--index', String(index)],
                    t('Removing source', '正在移除来源'),
                  ),
                )
              "
              ><Trash2
            /></Button>
          </div>
          <div class="row">
            <select v-model="provider">
              <option>modrinth</option>
              <option>curseforge</option>
              <option>file</option></select
            ><input
              v-model="locator"
              class="grow"
              :placeholder="
                provider === 'file'
                  ? t('JAR path', 'JAR 路径')
                  : t('Project ID', '项目 ID')
              "
            /><Button
              :disabled="!locator.trim()"
              @click="
                act(async () => {
                  await mutate(
                    ['remote', 'add', pkg.mod_id, provider, locator],
                    t('Adding source', '正在添加来源'),
                  );
                  locator = '';
                })
              "
              ><Plus />{{ t("Add", "添加") }}</Button
            >
          </div>
        </div>
        <div class="card danger-zone row wrap">
          <div class="grow">
            <h3>{{ t("Remove mod and data", "移除模组及数据") }}</h3>
            <p>
              {{
                t(
                  "Review all affected files before removal.",
                  "删除前会展示全部受影响文件。",
                )
              }}
            </p>
          </div>
          <Button
            @click="
              act(async () => {
                await mutate(
                  ['purge', pkg.mod_id],
                  t('Removing mod and data', '正在移除模组及数据'),
                );
                emit('close');
              })
            "
            >{{ t("Purge…", "彻底移除…") }}</Button
          >
        </div>
      </template>
    </div>
    <template v-if="['versions', 'strings'].includes(tab)" #footer
      ><span class="grow muted">{{
        t(
          "No changes until you apply and confirm.",
          "应用并确认方案之前不会更改实例。",
        )
      }}</span
      ><Button
        type="colored"
        :disabled="!versions || loading || saving || mode === 'custom'"
        @click="act(apply)"
        >{{ t("Apply policy", "应用策略") }}</Button
      ></template
    >
  </Modal>
</template>
