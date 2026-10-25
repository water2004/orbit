# Modrinth App adaptation

Base: https://github.com/modrinth/code at `4ccf4d51804627cd0687ce7dfd5004f20c267acc`.

The Vue application adapts Modrinth App's navigation rail, library/card layout,
instance content navigation and shared control styling. The following source
components are vendored and adapted under GPL-3.0-only (see LICENSE):

- `src/components/upstream/NavButton.vue`: apps/app-frontend/src/components/ui/NavButton.vue
- `src/components/upstream/InstanceCard.vue`: apps/app-frontend/src/components/ui/library/instance-group/instance-card-view.vue
- `src/components/upstream/{Button.vue,ButtonFrame.vue,types.ts}`: packages/ui/src/components/base/buttons/

Copyright belongs to the original Modrinth contributors. Orbit adaptations:
copyright 2026 Orbit contributors. Modrinth branding is not included. Orbit's
logo remains the original Orbit asset. Icons use Lucide under ISC.

Modrinth's Tauri commands, authentication, network clients, hosting, social,
analytics and advertising services are not imported. `src/services/` adapts
the existing Orbit/Launcher CLI protocol. The Rust process bridge is retained.
