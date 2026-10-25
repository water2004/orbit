# Modrinth App adaptation

Base: https://github.com/modrinth/code at `4ccf4d51804627cd0687ce7dfd5004f20c267acc`.

The Vue application adapts Modrinth App's navigation rail, library/card layout,
instance content navigation and shared control styling. The following source
components are vendored and adapted under GPL-3.0-only (see LICENSE):

- `src/components/upstream/NavButton.vue`: apps/app-frontend/src/components/ui/NavButton.vue
- `src/components/upstream/InstanceCard.vue`: apps/app-frontend/src/components/ui/library/instance-group/instance-card-view.vue
- `src/components/upstream/{Button.vue,ButtonFrame.vue,types.ts}`: packages/ui/src/components/base/buttons/
- `src/components/upstream/BigOptionButton.vue`: packages/ui/src/components/base/BigOptionButton.vue
- `src/components/upstream/Toggle.vue`: packages/ui/src/components/base/Toggle.vue

Interaction references at the same upstream revision:

- `apps/app-frontend/src/providers/content-install.ts` and `pages/Browse.vue`:
  instance-scoped discovery, per-project installing/installed state and returning to content.
- `apps/app-frontend/src/pages/instance/layout.vue` and `pages/instance/content/index.vue`:
  instance context, content actions and task-specific dialogs.
- `packages/ui/src/components/flows/creation-flow-modal/components/SetupTypeStage.vue`:
  custom installation / modpack / existing-directory entry choices.
- `apps/app-frontend/src/features/minecraft-accounts/accounts-card.vue`:
  quick account switching without leaving the current task.
- `packages/assets/styles/variables.scss`: neutral surface hierarchy for light/dark themes.

Orbit preserves its own CLI-driven dependency choices and final transaction confirmation;
it does not copy Modrinth's installation backend or infer compatibility in the frontend.

Copyright belongs to the original Modrinth contributors. Orbit adaptations:
copyright 2026 Orbit contributors. Modrinth branding is not included. Orbit's
logo remains the original Orbit asset. Icons use Lucide under ISC.

Modrinth's Tauri commands, authentication, network clients, hosting, social,
analytics and advertising services are not imported. `src/services/` adapts
the existing Orbit/Launcher CLI protocol. The Rust process bridge is retained.
