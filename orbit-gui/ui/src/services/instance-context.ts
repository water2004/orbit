import { inject, reactive, type InjectionKey, type ShallowRef } from "vue";
import type { Detail, Instance, Package } from "../types";
import { t } from "../i18n";
import { launcher, orbit } from "./workspace";
import type { Task } from "./cli";

// A page owns this view of exactly one instance. It is never a global selection
// and cannot retarget an already-started request when another page opens.
export function createInstanceContext(instance: Instance) {
  // Keep command identity separate from mutable presentation metadata.
  const target = { ...instance };
  const state = reactive({
    instance: { ...instance },
    detail: null as Detail | null,
    packages: [] as Package[],
    inventoryError: "",
    error: "",
    loading: false,
  });
  let revision = 0;
  let disposed = false;
  async function refresh() {
    if (disposed) return;
    const request = ++revision;
    state.loading = true;
    state.error = "";
    try {
      const detail = await launcher<Detail>(
        ["instance", "show"],
        t("Loading installation", "正在加载实例"),
        target,
      );
      if (request !== revision) return;
      if (detail.id !== target.id)
        throw new Error(
          t("CLI returned a different instance", "CLI 返回了其他实例的数据"),
        );
      state.detail = detail;
      state.inventoryError = "";
      if (detail.desired.loader === "vanilla") {
        state.packages = [];
        return;
      }
      try {
        const inventory = await orbit<{ packages: Package[] }>(
          ["list"],
          t("Loading mods", "正在加载模组"),
          target,
        );
        if (request === revision) state.packages = inventory.packages;
      } catch (error) {
        if (request === revision) {
          state.packages = [];
          state.inventoryError = String(error);
        }
      }
    } catch (error) {
      if (request === revision) state.error = String(error);
      throw error;
    } finally {
      if (request === revision) state.loading = false;
    }
  }
  return {
    state,
    refresh,
    dispose() {
      disposed = true;
      ++revision;
    },
    orbit: <T>(
      args: string[],
      label: string,
      instance: Instance | null = target,
      observe?: (task: Task) => void,
    ) => orbit<T>(args, label, instance, observe),
    launcher: <T>(
      args: string[],
      label: string,
      instance: Instance | null = target,
      observe?: (task: Task) => void,
    ) => launcher<T>(args, label, instance, observe),
    async mutate(
      args: string[],
      label: string,
      observe?: (task: Task) => void,
    ) {
      await orbit(args, label, target, observe);
      await refresh();
    },
  };
}

export type InstanceContext = ReturnType<typeof createInstanceContext>;
export const instanceContextKey: InjectionKey<
  ShallowRef<InstanceContext | undefined>
> = Symbol("instance-page-context");
export function useInstanceContext(): InstanceContext {
  const context = inject(instanceContextKey)?.value;
  if (!context)
    throw new Error("Instance page requires an explicit route context");
  return context;
}
