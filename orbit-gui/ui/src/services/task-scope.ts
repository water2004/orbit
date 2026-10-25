import { shallowRef } from "vue";
import { run, type Request, type Task } from "./cli";
import { workspace, launcher, orbit, mutate } from "./workspace";

// Display ownership only: these wrappers use the same CLI calls and protocol.
// Capturing the task at spawn prevents an unrelated background request from
// stealing a dialog's progress, including requests which finish immediately.
export function useCommandScope() {
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
      instance = workspace.selected,
    ) => launcher<T>(args, label, instance, observe),
    orbit: <T>(args: string[], label: string, instance = workspace.selected) =>
      orbit<T>(args, label, instance, observe),
    mutate: (args: string[], label: string, instance = workspace.selected) =>
      mutate(args, label, instance, observe),
  };
}
