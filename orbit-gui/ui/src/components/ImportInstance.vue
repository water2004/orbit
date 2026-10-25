<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { FolderOpen } from "lucide-vue-next";
import { useCommandScope } from "../services/task-scope";
import { refresh, selectInstance } from "../services/workspace";
import type { Detail, Instance } from "../types";
import { Cancelled } from "../services/cli";
import { t } from "../i18n";
import Button from "./upstream/Button.vue";
import DropdownSelect from "./upstream/DropdownSelect.vue";
import Modal from "./Modal.vue";
interface Source {
  id: string;
  name: string;
  format: string;
  kind: string;
  profile_directory: string;
  game_directory?: string;
  runtime?: { minecraft: string; loader: string; loader_version?: string };
  directory_evidence: string;
  warnings: string[];
  error?: string;
}
interface Preview {
  game_directory: string;
  destination: string;
  files: number;
  bytes: number;
  excluded: string[];
  resuming: boolean;
}
const emit = defineEmits<{ close: []; imported: [id: string] }>();
const { launcher, task } = useCommandScope();
const directory = ref(""),
  sources = ref<Source[]>([]),
  sourceId = ref(""),
  name = ref(""),
  game = ref(""),
  busy = ref(false),
  error = ref(""),
  preview = ref<Preview>();
const source = computed(() =>
  sources.value.find((s) => s.id === sourceId.value),
);
const orbitSource = computed(() => source.value?.format === "orbit");
watch(source, (s) => {
  name.value = s?.name || "";
  game.value = s?.game_directory || "";
  preview.value = undefined;
});
watch([name, game], () => {
  preview.value = undefined;
});
async function act(operation: () => Promise<void>) {
  busy.value = true;
  error.value = "";
  try {
    await operation();
  } catch (e) {
    if (!(e instanceof Cancelled)) error.value = String(e);
  } finally {
    busy.value = false;
  }
}
async function choose() {
  const path = await open({ directory: true, multiple: false });
  if (typeof path !== "string") return;
  directory.value = path;
  sources.value = [];
  sourceId.value = "";
  preview.value = undefined;
  const catalog = await launcher<{ instances: Source[] }>(
    ["instance", "inspect", "--directory", path],
    t("Inspecting existing instances", "正在检查已有实例"),
    null,
  );
  sources.value = catalog.instances;
  if (catalog.instances.length === 1) sourceId.value = catalog.instances[0].id;
}
function args() {
  return [
    "instance",
    "import",
    "--directory",
    directory.value,
    ...(orbitSource.value
      ? []
      : [
          "--source",
          sourceId.value,
          "--name",
          name.value.trim(),
          "--game-directory",
          game.value,
        ]),
  ];
}
async function review() {
  preview.value = await launcher<Preview>(
    [...args(), "--dry-run"],
    t("Reviewing instance import", "正在准备实例导入预览"),
    null,
  );
}
async function apply() {
  const result = await launcher<{ instance: Instance }>(
    args(),
    t("Importing instance", "正在导入实例"),
    null,
  );
  const id = result.instance.id;
  await refresh();
  const detail = await launcher<Detail>(
    ["instance", "show"],
    t("Loading imported instance", "正在加载导入的实例"),
    { id } as Instance,
  );
  await selectInstance(detail);
  emit("imported", id);
}
</script>
<template>
  <Modal
    :title="t('Import an existing instance', '导入已有实例')"
    :task="task"
    :closable="!busy"
    @close="emit('close')"
  >
    <div class="stack">
      <p>
        {{
          t(
            "Choose a launcher game folder or an isolated instance. Original files are kept.",
            "选择启动器的游戏根目录或独立实例目录，原文件会保留。",
          )
        }}
      </p>
      <div class="row">
        <span class="grow path">{{
          directory || t("No source selected", "尚未选择来源")
        }}</span
        ><Button :disabled="busy" @click="act(choose)"
          ><FolderOpen />{{ t("Choose folder…", "选择目录…") }}</Button
        >
      </div>
      <label v-if="sources.length" class="field"
        >{{ t("Source instance", "来源实例")
        }}<DropdownSelect
          v-model="sourceId"
          :disabled="busy"
          :label="t('Source instance', '来源实例')"
          :options="
            sources.map((s) => ({
              value: s.id,
              label: s.name,
              description: s.runtime
                ? `${s.runtime.minecraft} · ${s.runtime.loader} · ${s.format}`
                : s.error || s.format,
            }))
          "
      /></label>
      <template v-if="source">
        <p v-if="source.error" class="error">{{ source.error }}</p>
        <p v-if="orbitSource">
          {{
            t(
              "This is an Orbit instance. Register its existing directory without copying or reinstalling.",
              "这是 Orbit 实例，将重新注册现有目录，不复制或重新安装。",
            )
          }}
        </p>
        <template v-else-if="source.runtime">
          <div class="card">
            <strong>Minecraft {{ source.runtime.minecraft }}</strong>
            <p>
              {{ source.runtime.loader }} {{ source.runtime.loader_version }}
            </p>
          </div>
          <label class="field"
            >{{ t("New instance name", "新实例名称")
            }}<input v-model="name" :disabled="busy"
          /></label>
          <label class="field"
            >{{ t("Game data directory", "游戏数据目录") }}
            <div class="row">
              <span class="grow path">{{
                game ||
                t("Confirm the actual game directory", "请确认实际游戏运行目录")
              }}</span
              ><Button
                :disabled="busy"
                @click="
                  act(async () => {
                    const path = await open({ directory: true });
                    if (typeof path === 'string') game = path;
                  })
                "
                ><FolderOpen />{{ t("Choose…", "选择…") }}</Button
              >
            </div></label
          >
          <small>{{
            t(
              "If the source inherits unavailable global settings, choose its actual working directory containing mods, saves and options.",
              "若来源继承了无法取得的全局设置，请选择实际存放模组、存档和选项的运行目录。",
            )
          }}</small>
          <p>
            {{
              t(
                "Minecraft and Loader are installed from official metadata. Game data, including mods and worlds, is copied as-is; no dependency solving is performed.",
                "Minecraft 和加载器按官方元数据安装。模组、存档等游戏数据原样复制，不执行依赖求解。",
              )
            }}
          </p>
          <details>
            <summary>
              {{ t("What is not imported?", "哪些内容不会导入？") }}
            </summary>
            <p>
              {{
                t(
                  "Accounts, login tokens, Java overrides, JVM options and launch hooks. Shared libraries/assets are reinstalled, not copied as game data.",
                  "账户、登录令牌、Java 覆盖、JVM 参数和启动钩子不导入；共享运行库和资源重新安装，不作为游戏数据复制。",
                )
              }}
            </p>
          </details>
        </template>
      </template>
      <div v-if="preview" class="card stack">
        <strong>{{
          preview.resuming
            ? t("Resume unfinished import", "继续未完成的导入")
            : t("Ready to import", "导入预览")
        }}</strong>
        <p class="path">
          {{ preview.game_directory }} → {{ preview.destination }}
        </p>
        <p>
          {{ preview.files }} {{ t("files", "个文件") }} ·
          {{ (preview.bytes / 1048576).toFixed(1) }} MiB
        </p>
        <details v-if="preview.excluded.length">
          <summary>
            {{
              t(
                "Excluded runtime and launcher files",
                "不复制的运行时与启动器文件",
              )
            }}
          </summary>
          <ul>
            <li v-for="path in preview.excluded" :key="path">{{ path }}</li>
          </ul>
        </details>
      </div>
      <p v-if="error" role="alert" class="error">{{ error }}</p>
    </div>
    <template #footer
      ><Button :disabled="busy" @click="emit('close')">{{
        t("Cancel", "取消")
      }}</Button
      ><Button
        v-if="orbitSource || preview"
        type="colored"
        :loading="busy"
        @click="act(apply)"
        >{{
          orbitSource
            ? t("Register instance", "注册实例")
            : t("Import instance", "开始导入")
        }}</Button
      ><Button
        v-else
        type="colored"
        :loading="busy"
        :disabled="!source?.runtime || !name.trim() || !game"
        @click="act(review)"
        >{{ t("Review import", "预览导入") }}</Button
      ></template
    >
  </Modal>
</template>
<style scoped>
.path {
  overflow-wrap: anywhere;
  min-width: 0;
  font-size: 13px;
}
small {
  line-height: 1.6;
}
</style>
