import { t } from "../i18n";

// Presentation of protocol identifiers only; account selection stays in Launcher.
export function accountProviderLabel(provider: string, endpoint?: string) {
  switch (provider) {
    case "microsoft":
      return "Microsoft";
    case "offline":
      return t("Offline", "离线账户");
    case "external-yggdrasil":
      return endpoint || "Yggdrasil";
    default:
      return provider;
  }
}
