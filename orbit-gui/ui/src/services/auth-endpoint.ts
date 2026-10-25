import { shallowRef } from "vue";

// Modal presentation state only. Raw transport text is never decoded here.
export const endpointDialog = shallowRef<{
  key: number;
  address: string;
  added?: (id: string) => Promise<void>;
}>();
let key = 0;
export function openAuthEndpoint(
  address = "",
  added?: (id: string) => Promise<void>,
) {
  endpointDialog.value = { key: ++key, address, added };
}
