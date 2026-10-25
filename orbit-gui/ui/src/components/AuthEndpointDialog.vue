<script setup lang="ts">
import { onMounted, onUnmounted, ref, watch } from "vue";
import { useCommandScope } from "../services/task-scope";
import { Cancelled } from "../services/cli";
import { t } from "../i18n";
import Modal from "./Modal.vue";
import Button from "./upstream/Button.vue";

interface Endpoint {
  api_root: string;
  server_name?: string;
  suggested_id: string;
  existing_provider_id?: string;
  insecure_http: boolean;
}
const props = defineProps<{ address?: string }>();
const emit = defineEmits<{
  close: [];
  added: [id: string];
  busy: [value: boolean];
}>();
const { launcher, task } = useCommandScope();
const address = ref(props.address || ""),
  id = ref(""),
  insecure = ref(false);
const preview = ref<Endpoint>(),
  busy = ref(false),
  error = ref("");
watch(busy, (value) => emit("busy", value), { flush: "sync" });
let revision = 0;
watch([address, insecure], () => {
  revision++;
  preview.value = undefined;
  error.value = "";
});
onUnmounted(() => {
  revision++;
});
async function inspect() {
  if (busy.value || !address.value.trim()) return;
  const current = ++revision;
  busy.value = true;
  error.value = "";
  preview.value = undefined;
  try {
    const result = await launcher<Endpoint>(
      [
        "config",
        "yggdrasil",
        "inspect",
        address.value,
        ...(insecure.value ? ["--allow-insecure-http"] : []),
      ],
      t("Checking authentication endpoint", "正在检查认证端点"),
      null,
    );
    if (current === revision) {
      preview.value = result;
      if (!id.value)
        id.value = result.existing_provider_id || result.suggested_id;
    }
  } catch (e) {
    if (current === revision && !(e instanceof Cancelled))
      error.value = String(e);
  } finally {
    busy.value = false;
  }
}
async function save() {
  const endpoint = preview.value;
  if (busy.value || !endpoint || !id.value.trim()) return;
  if (endpoint.existing_provider_id) {
    emit("added", endpoint.existing_provider_id);
    return;
  }
  busy.value = true;
  error.value = "";
  try {
    const result = await launcher<{ provider: { id: string } }>(
      [
        "config",
        "yggdrasil",
        "add",
        id.value,
        endpoint.api_root,
        ...(insecure.value ? ["--allow-insecure-http"] : []),
      ],
      t("Saving authentication endpoint", "正在保存认证端点"),
      null,
    );
    emit("added", result.provider.id);
  } catch (e) {
    if (!(e instanceof Cancelled)) error.value = String(e);
  } finally {
    busy.value = false;
  }
}
onMounted(() => {
  if (props.address) void inspect();
});
</script>
<template>
  <Modal
    :title="t('Add authentication endpoint', '添加认证端点')"
    :task="task"
    :closable="!busy"
    @close="emit('close')"
  >
    <div class="stack">
      <p>
        {{
          t(
            "Drag a skin site's setup button into Orbit, or paste its website, API address or setup link here.",
            "可以将皮肤站的配置按钮拖入 Orbit，也可在此粘贴网站、API 地址或配置链接。",
          )
        }}
      </p>
      <label class="field"
        >{{
          t("Website, API address or setup link", "网站、API 地址或配置链接")
        }}
        <input v-model="address" :disabled="busy" placeholder="https://" />
      </label>
      <label
        ><input v-model="insecure" type="checkbox" :disabled="busy" />{{
          t("Allow unencrypted HTTP (unsafe)", "允许未加密 HTTP（不安全）")
        }}</label
      >
      <p v-if="insecure" role="alert" class="error">
        {{
          t(
            "HTTP sends passwords without encryption; only add this service if you accept the risk.",
            "HTTP 会明文传输密码；仅在你接受此风险时添加。",
          )
        }}
      </p>
      <Button
        :disabled="busy || !address.trim()"
        :loading="busy && !preview"
        @click="inspect"
        >{{ t("Check service", "检查服务") }}</Button
      >
      <div v-if="preview" class="card stack endpoint-preview">
        <strong>{{ preview.server_name || preview.suggested_id }}</strong>
        <p class="endpoint-address">{{ preview.api_root }}</p>
        <p>
          {{
            t(
              "Confirm that you trust this authentication service before adding it. No account credentials have been sent.",
              "请确认你信任此认证服务后再添加。此时尚未发送任何账户凭据。",
            )
          }}
        </p>
        <p v-if="preview.existing_provider_id">
          {{ t("This endpoint is already configured.", "此端点已配置。") }}
        </p>
        <label v-else class="field"
          >{{ t("Name", "名称") }}<input v-model="id" :disabled="busy"
        /></label>
      </div>
      <p v-if="error" role="alert" class="error">{{ error }}</p>
    </div>
    <template #footer>
      <Button :disabled="busy" @click="emit('close')">{{
        t("Cancel", "取消")
      }}</Button>
      <Button
        type="colored"
        :disabled="busy || !preview || !id.trim()"
        @click="save"
      >
        {{
          preview?.existing_provider_id
            ? t("Use existing service", "使用已有服务")
            : t("Add service", "添加服务")
        }}
      </Button>
    </template>
  </Modal>
</template>
<style scoped>
.endpoint-address {
  overflow-wrap: anywhere;
}
.endpoint-preview {
  border: 1px solid var(--surface-4);
}
</style>
