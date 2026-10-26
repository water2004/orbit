<p align="center">
  <img src="assets/orbit.svg" width="96" height="96" alt="Orbit logo">
</p>

# Orbit

[简体中文](README.zh-CN.md) · [Downloads](https://github.com/water2004/orbit/releases) · [Documentation](docs/orbit-cli-commands.md)

**Minecraft mod management, runtime management, and a desktop workspace—with clear boundaries.**

Orbit is still a non-intrusive mod package manager: it works with an existing valid Minecraft
instance without requiring you to switch launchers. The suite also includes its own independent
Launcher and a graphical interface for both CLIs.

| Application | Responsibility |
| --- | --- |
| **Orbit** (`orbit`) | Mods, JAR-declared dependencies, constraints, repairs, updates, exact restoration, compatibility audit and package data. |
| **Orbit Launcher** (`orbit-launcher`) | Minecraft, Loaders, Java, accounts, isolated instances, launching and supervised servers. No mod resolution or provider integration. |
| **Orbit GUI** (`orbit-gui`) | An instance-oriented Vue/Tauri desktop interface. All business operations use the same CLI commands and machine protocol as terminal users. |

Launcher never links to or calls Orbit. For package data tracking, Orbit can wrap Launcher's
existing launch command and inject its own runtime agent. Both CLIs also work independently.

## Desktop workspace

An instance library, instance-bound mod discovery, package/version management, accounts, Java
selection, guided installation, imports/exports and migration are available from the desktop.
Long-running work exposes real progress, cancellation and explicit plan choices.

![Orbit instance library](assets/screenshots/instance-library.png)

![Minecraft and Loader selection](assets/screenshots/create-instance.png)

*Actual frontend captures in the Chinese locale, using isolated demonstration data—not design
mockups or a running game. The interface also supports English and system language selection.*

## What Orbit does

- **Trust the JAR, not a provider's labels.** Package IDs, versions, dependencies, environment and
  bundled content come from Loader metadata. Modrinth and CurseForge locate downloadable files;
  one logical package can have several remotes, including local files.
- **Keep sync, restore and repair separate.** `sync` records the local state and can recover
  sources online; `install` restores an exact lock; `fix` solves dependency problems. Neither sync
  nor install secretly repairs the instance.
- **Offer real choices.** Add/fix minimize changes. All-package updates seek the Pareto-maximal
  version front. A targeted update prioritizes that mod's highest feasible version, then minimizes
  changes to the other mods. Independent choices are factored rather than expanded into a huge
  Cartesian list. Necessary downgrades, replacements and removals are shown before confirmation.
- **Manage versions without editing TOML.** Inspect available versions, set numeric constraints,
  combine string filters, and enable/disable packages. The GUI provides visual rule editing.
- **Explain compatibility risks.** Audit uses Loader-specific runtime, namespace and Mixin rules.
  It is a static risk report, not a guarantee that a modpack will run correctly.
- **Handle data deliberately.** Optional runtime observation records successful writes without
  monitoring reads. Package files/data can be inspected, reset, purged or included in migration;
  unknown ownership is preserved and external paths are excluded from portable migration.
- **Avoid repeated analysis.** Game/Loader-scoped provider and JAR-analysis databases are separate
  from the global, hash-addressed LRU JAR cache.

## What Launcher does

- Install Vanilla, Fabric, Quilt, Forge and NeoForge from their metadata/installer adapters,
  subject to the supported version ranges.
- Create isolated clients under `instances/<name>`, with their own Minecraft JAR and mutable
  game data; share immutable libraries/assets. Dedicated servers use an explicit server directory.
- Prefer an appropriate **local Java** in automatic mode; install Mojang Java only when needed.
  Global and per-instance selections can be set explicitly.
- Support Microsoft, offline and standard External Yggdrasil accounts. Tokens use the operating
  system's secret store; the Microsoft public-client registration is built in.
- Launch clients with configurable resolution, and supervise dedicated servers with optional
  crash restart. The full current server EULA must be shown and explicitly accepted.
- **Import existing clients** from standard/official profiles, HMCL, PCL, Prism/MultiMC,
  CurseForge, GDLauncher Carbon, ATLauncher and supported Modrinth App layouts. Preview the
  actual game directory, copy into a new isolated instance and keep the source untouched.
  Unavailable global isolation settings require an explicit directory choice, not a guess.
- **Delete an entire instance**, including its worlds and mods, as a separate confirmed action
  from unregistering it. System trash is used; failure does not silently become permanent deletion.

Foreign import copies mods as opaque user files—it does not manage or solve them. Customized
runtime formats and arbitrary server distributions are not automatically converted. See
[import support and safety](docs/orbit-launcher-import.md) for the exact scope and recovery rules.

## Install

Get packages from [GitHub Releases](https://github.com/water2004/orbit/releases).

- **Windows x64:** one MSI, with Orbit-only, Orbit + Launcher, or the complete suite. Optional
  CLI `PATH` integration, same-version maintenance and uninstall data choices are included.
- **Debian/Ubuntu amd64:** separate `orbit`, `orbit-launcher` and `orbit-gui` debs. Install only
  Launcher on a headless server, and add Orbit for mod management. The GUI package depends on
  matching versions of both CLIs and needs a graphical session.

The GUI uses WebView2 on Windows and WebKitGTK 4.1 on Linux. Service installation, startup at
boot and operation after logout are not part of the server supervisor.

## Quick start

### Manage mods in an existing game directory

```sh
cd /path/to/your/game-directory
orbit init survival
orbit sync
orbit add sodium
orbit package sodium versions
orbit upgrade sodium
```

Useful commands:

| Command | Effect |
| --- | --- |
| `orbit sync` | Rebuild facts from the actual runtime and local JARs; identify known remotes online. No solving or JAR deletion. |
| `orbit fix` | Find and confirm a minimal-change dependency repair. |
| `orbit install` | Restore the exact existing lockfile, without solving. |
| `orbit outdated` / `orbit upgrade` | Inspect/apply global updates, including reasons for blocked upgrades. |
| `orbit upgrade <mod_id>` | Prioritize that package's update and minimize other changes. |
| `orbit package <mod_id> constraint set …` | Solve and apply a version policy transaction; cancellation/failure leaves the old policy intact. |
| `orbit package <mod_id> disable` / `orbit package <mod_id> enable` | Toggle Loader discovery by renaming the selected JAR and recording its state. |
| `orbit audit` | Read-only bytecode compatibility risk analysis. |
| `orbit package <mod_id> reset` / `orbit package <mod_id> purge` | Confirm a data-only reset, or remove the package and its observed data. |

### Create or import an instance

```sh
# A new isolated client; use the GUI or versions commands to choose exact versions.
orbit-launcher install --new survival --kind client \
  --minecraft latest-release --loader fabric --loader-version stable

# Inspect and preview an existing launcher's client before copying it.
orbit-launcher instance inspect --directory /path/to/source
orbit-launcher instance import --directory /path/to/source \
  --source <id-from-inspect> --name imported --dry-run
# Repeat without --dry-run to perform the import.

# A dedicated server (explicit EULA acceptance is required).
orbit-launcher install --new server --kind server --server-directory /srv/minecraft \
  --minecraft latest-release --loader fabric --loader-version stable
```

`instance unregister` only unregisters an instance. Use `instance delete --dry-run` to review full
directory deletion; the terminal asks for confirmation unless `--yes` is supplied.

### Packs and migration

`.orbitbundle` is one versioned format with optional, disjoint Orbit and Launcher sections.
Orbit owns mods, policies, lock state and selected package data; Launcher owns runtime
requirements, worlds and game preferences. The GUI combines their existing commands to export
and migrate to a **new** instance. Official Modrinth `.mrpack` import/export is also supported.

Migration first attempts to keep every source package. If that is impossible, the user can opt
into a minimal-removal search. Existing instances are not silently overwritten or upgraded in place.

## Configuration and automation

`orbit.toml` records every managed top-level logical package and its policies; `orbit.lock`
records the exact selected state. Commit both for reproducible restoration. Every package has
equal standing—there is no root/transitive package split.

CurseForge requires a user-owned API key; there is no anonymous fallback:

```sh
orbit config set auth.curseforge-api-key YOUR_API_KEY
```

Alternatively use `ORBIT_CURSEFORGE_API_KEY` to avoid putting the key in shell history.
Global settings, cache capacity and Java can be managed through the CLIs or the GUI.

Both CLIs support `--language system|en|zh-CN` (default `system`), `--output-format json` and
`--progress-format ndjson`. Structured stdout/stderr and interactive stdin use strict UTF-8;
the GUI does not use an alternate business API. Released Launcher storage from v0.7.0 onward is
upgraded by a dedicated, ordered migration module, with backups before replacement.

## Documentation and development

- [Orbit commands](docs/orbit-cli-commands.md) · [Launcher commands](docs/orbit-launcher-cli.md)
- [Existing-instance import and deletion](docs/orbit-launcher-import.md)
- [Manifest schema](docs/orbit-toml-spec.md) · [Version rules](docs/orbit-versions.md)
- [Portable pack format](docs/orbit-bundle-format.md) · [Bytecode audit](docs/orbit-bytecode-audit.md)
- [GUI architecture and build](docs/orbit-gui.md) · [Project architecture](docs/orbit-architecture.md)
- [Windows MSI](docs/windows-msi.md) · [Linux debs](docs/linux-deb.md) · [Release process](docs/release-process.md)

```sh
npm --prefix orbit-gui/ui ci
npm --prefix orbit-gui/ui test
npm --prefix orbit-gui/ui run build
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Follow [project conventions](CLAUDE.md), keep the CLI/core/wrapper boundaries intact, and test
Loader-specific behavior. The [PubGrub fork](https://github.com/water2004/pubgrub) supplies the
solver primitives; solver policy does not live in the GUI.

## License

Orbit and Orbit Launcher are MIT. The GUI adapts Modrinth App components and is GPL-3.0-only;
see [GUI license](orbit-gui/ui/LICENSE), [upstream attribution](orbit-gui/ui/UPSTREAM.md), and
[screenshot provenance](assets/screenshots/README.md).
