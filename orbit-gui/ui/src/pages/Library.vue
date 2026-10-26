<script setup lang="ts">
import { computed, ref } from "vue";
import { useRouter } from "vue-router";
import { Plus, Search, Play, FolderInput, Settings } from "lucide-vue-next";
import { workspace, launch, act } from "../services/workspace";
import { t } from "../i18n";
import { instancePath } from "../services/navigation";
import Button from "../components/upstream/Button.vue";
import InstanceCard from "../components/upstream/InstanceCard.vue";
const router = useRouter();
const query = ref("");
const kind = ref("all");
const compact = ref(false);
const instances = computed(() =>
  workspace.instances.filter(
    (i) =>
      (kind.value === "all" || i.kind === kind.value) &&
      i.name.toLowerCase().includes(query.value.toLowerCase()),
  ),
);
async function openInstance(id: string) {
  const instance = workspace.instances.find((i) => i.id === id);
  if (instance) {
    await router.push(instancePath(instance.id));
  }
}
</script>
<template>
  <div class="page">
    <div class="page-heading">
      <div>
        <h1>{{ t("Your library", "实例库") }}</h1>
        <p>
          {{
            t(
              "Pick a world to return to, or start something new.",
              "继续你的冒险，或从一个新实例开始。",
            )
          }}
        </p>
      </div>
      <Button type="colored" @click="router.push('/create')"
        ><Plus />{{ t("Create instance", "创建实例") }}</Button
      >
    </div>
    <div class="toolbar">
      <Search :size="18" /><input
        v-model="query"
        class="search"
        :placeholder="t('Search instances', '搜索实例')"
      />
      <div class="tabs">
        <button
          v-for="k in ['all', 'client', 'server']"
          :key="k"
          :class="{ active: kind === k }"
          @click="kind = k"
        >
          {{
            k === "all"
              ? t("All", "全部")
              : k === "client"
                ? t("Clients", "客户端")
                : t("Servers", "服务器")
          }}
        </button>
      </div>
      <label
        ><input v-model="compact" type="checkbox" />{{
          t("Compact", "紧凑")
        }}</label
      >
    </div>
    <TransitionGroup
      name="list"
      tag="div"
      class="instance-grid"
      :class="{ compact }"
    >
      <article
        v-for="instance in instances"
        :key="instance.id"
        class="instance-item"
      >
        <button class="card-link" @click="act(() => openInstance(instance.id))">
          <InstanceCard :instance="instance" :compact="compact" />
        </button>
        <div class="instance-actions">
          <Button type="colored" size="sm" @click="act(() => launch(instance))"
            ><Play />{{
              instance.kind === "server"
                ? t("Start", "启动")
                : t("Play", "开始游戏")
            }}</Button
          ><Button
            type="quiet"
            size="sm"
            :title="t('Manage instance', '管理实例')"
            @click="
              act(async () => {
                await router.push(instancePath(instance.id, 'runtime'));
              })
            "
            ><Settings
          /></Button>
        </div>
      </article>
    </TransitionGroup>
    <div v-if="!instances.length" class="empty">
      <h2>
        {{
          workspace.loading
            ? t("Loading your library…", "正在加载实例库…")
            : t("Make room for your next adventure", "开始下一段冒险")
        }}
      </h2>
      <p>
        {{
          t(
            "Create an instance, install a pack, or import a game directory.",
            "创建实例、安装整合包，或导入现有游戏目录。",
          )
        }}
      </p>
    </div>
    <div class="row">
      <small class="grow"
        >{{ instances.length }} {{ t("instances", "个实例") }}</small
      ><Button type="quiet" @click="router.push('/create?import=1')"
        ><FolderInput />{{ t("Import existing", "导入现有实例") }}</Button
      ><Button @click="router.push('/create?pack=1')">{{
        t("Install modpack", "安装整合包")
      }}</Button>
    </div>
  </div>
</template>
<style scoped>
.instance-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(210px, 1fr));
  gap: 18px;
  align-items: start;
}
.instance-grid.compact {
  grid-template-columns: repeat(auto-fill, minmax(310px, 1fr));
}
.instance-item {
  position: relative;
}
.card-link {
  display: block;
  text-align: left;
  width: 100%;
  border: 0;
  background: none;
  border-radius: 20px;
  cursor: pointer;
  transition: filter var(--motion-fast);
}
.card-link:hover {
  filter: brightness(1.05);
}
.instance-actions {
  display: flex;
  justify-content: space-between;
  padding: 8px 4px;
}
.compact .instance-actions {
  justify-content: flex-end;
}
</style>
