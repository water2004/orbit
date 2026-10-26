import { shallowRef } from "vue";
import { run, type Request, type Task } from "./cli";
import { launcher, orbit } from "./workspace";
import type { Instance } from "../types";

// Display ownership only: these wrappers use the same CLI calls and protocol.
// Capturing the task at spawn prevents an unrelated background request from
// stealing a dialog's progress, including requests which finish immediately.
export function useCommandScope(
  target: Instance | null = null,
  changed?: () => Promise<void>,
) {
  const task = shallowRef<Task>();
  const observe = (value: Task) => {
    task.value = value;
  };
  return {
    task,
    run: <T>(request: Request) => run<T>(request, observe),
    launcher: <T>(
      args: string[],
      label: string,
      instance: Instance | null = target,
    ) => launcher<T>(args, label, instance, observe),
    orbit: <T>(
      args: string[],
      label: string,
      instance: Instance | null = target,
    ) => orbit<T>(args, label, instance, observe),
    mutate: async (args: string[], label: string) => {
      if (!target)
        throw new Error("Package mutations require an explicit instance");
      await orbit(args, label, target, observe);
      await changed?.();
    },
  };
}
