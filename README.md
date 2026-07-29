<p align="center">
  <img src="assets/orbit.svg" width="112" height="112" alt="Orbit logo">
</p>

# Orbit

[简体中文](README.zh-CN.md)

**A modern, non-intrusive package manager and native workspace for Minecraft Java Edition.**

Orbit brings package-manager semantics to Minecraft mods without replacing the launcher workflow you already use. It can adopt an existing valid game instance, reconstruct its package graph from JAR-declared metadata, discover candidates from multiple providers, resolve complete dependency portfolios, and restore an exact lockfile.

The repository contains three deliberately separated applications:

- `orbit` manages mods and their dependency graph.
- `orbit-launcher` manages Minecraft, Loaders, Java, accounts, launching, and supervised servers. It neither links to nor calls Orbit.
- `orbit-gui` is a native GPUI shell. It performs no package or launcher business logic and talks to the two CLIs exclusively through their JSON/NDJSON protocols.

The detailed boundaries are documented in [Orbit architecture](docs/orbit-architecture.md), [Launcher architecture](docs/orbit-launcher-architecture.md), and [GUI architecture](docs/orbit-gui.md).

## Highlights

- **Existing instances and isolated managed instances.** Orbit works in valid Fabric, Quilt, Forge, and NeoForge client or dedicated-server game directories. Launcher-managed clients use isolated `instances/<instance>` game directories; each owns its exact `minecraft.jar`, while immutable assets and libraries remain shared.
- **Truthful synchronization.** `orbit sync` redetects the runtime, scans local JARs, and uses provider hash APIs to recover sources. It rebuilds TOML and lock state without solving dependencies or deleting packages.
- **Explicit repair.** `orbit fix` is the command that discovers the full recursive candidate closure, resolves it, presents every package-level action, and applies a confirmed repair.
- **Multiple remotes per package.** A logical package can use local files, Modrinth projects, and CurseForge projects together. Content is deduplicated by hash; identity, version, dependencies, environment, `provides`, and bundled content come only from downloaded JAR metadata.
- **Loader-faithful metadata and audit backends.** Fabric, Quilt, Forge, and NeoForge keep their real registration, nesting, namespace, and Mixin activation rules before entering shared normalized models.
- **Explainable complete solving.** Dependency causes are emitted by the actual PubGrub propagation and backtracking path. Orbit enumerates the standard Pareto-maximal solution front and asks whenever more than one meaningful solution exists.
- **Package-level transactions.** `mod_id` is the solver package key. A selected package may own multiple contained JARs, while unselected top-level package versions are removed only after the exact plan is shown and confirmed.
- **Observable and cancellable work.** Discovery, downloads, solving, application, audit, and portable export emit typed progress. Orbit ZIP export stores already-compressed JARs directly, reports real byte progress, and cleans failed temporary output.
- **Portable migration snapshots.** The GUI exports a verified Orbit source pack before it creates a target runtime. The target migration then resolves from that frozen pack against the actually installed target Minecraft and Loader runtime.

## Installation

Windows x64 users can install the complete suite from `orbit-<version>-x86_64.msi` on the GitHub Releases page. The installer provides three feature profiles: Orbit only, Orbit + Launcher, or the complete suite with the native GUI. It can add the CLI directory to system `PATH`, supports same-version maintenance, and asks whether default AppData configuration and caches should be removed during uninstall.

Debian and Ubuntu amd64 releases contain three independently installable packages. A headless
server needs only Launcher, and can add Orbit when it also wants managed mods:

```bash
# Headless Minecraft runtime management.
sudo apt install ./orbit-launcher_0.2.0-1_amd64.deb

# Optional mod package management on that server.
sudo apt install ./orbit_0.2.0-1_amd64.deb

# Desktop installation: apt resolves the GUI's exact-version CLI dependencies
# when all three downloaded files are supplied together.
sudo apt install ./orbit_0.2.0-1_amd64.deb \
  ./orbit-launcher_0.2.0-1_amd64.deb \
  ./orbit-gui_0.2.0-1_amd64.deb
```

Installing the GUI on a headless host is technically harmless, but it pulls graphical runtime
libraries and cannot display without a graphical session, so the separate Launcher package is the
intended server installation.

Tagged releases are built only from `main` when the tag matches the Cargo version. See [release process](docs/release-process.md), [Windows MSI](docs/windows-msi.md), and [Linux deb](docs/linux-deb.md).

## Quick start

```bash
# Adopt an existing valid Minecraft game directory.
cd "D:/Games/HMCL/instances/MySurvival/.minecraft"
orbit init survival

# Add provider projects or a local JAR. CurseForge requires an API key.
orbit add sodium
orbit add cf:238222
orbit add file:./my-local-mod.jar

# Manage all candidate remotes of one logical package.
orbit remote add sodium modrinth AANobbMI
orbit remote add sodium curseforge 394468
orbit remote list sodium

# Optional root environment filtering; auto follows the selected JAR.
orbit env sodium client
orbit env sodium auto

# Repair, restore, update, and audit.
orbit fix
orbit install
orbit outdated
orbit upgrade
orbit audit
```

Both CLIs accept `--language system|en|zh-CN`; the default is `system`. Human-readable help, output, progress, prompts, and errors follow that setting. JSON, NDJSON, stdin responses, schema fields, enum codes, and error codes remain strict UTF-8 and language-neutral.

## Command model

Orbit resolves its instance from the current directory first, then an explicit `--instance`, then the read-only global default.

### Instance and package state

| Command | Responsibility |
| --- | --- |
| `orbit init <name>` | Initialize a valid game directory from redetected local facts. |
| `orbit sync` | Redetect the runtime, scan JARs, recover known remotes online, and rebuild TOML/lock facts without dependency solving. |
| `orbit fix` | Find and confirm a feasible package repair; this is the repair command. |
| `orbit install [--target client\|server]` | Materialize the exact existing lockfile. It does not solve, repair, remove packages, or rewrite state. |
| `orbit add <locator>` | Add a provider project or local file and solve the complete candidate closure. |
| `orbit remove <package>` | Remove the logical package and its TOML/lock state. |
| `orbit purge <package>` | Remove the package and interactively select related configuration candidates. |
| `orbit outdated [package]` | Explain feasible updates and why newer candidates are blocked. |
| `orbit upgrade [package]` | Apply a solution in which at least one requested package becomes newer; dependencies may downgrade or be replaced. |
| `orbit list [--tree]` | Show installed logical packages from the lockfile. |
| `orbit remote add/remove/list` | Manage local, Modrinth, and CurseForge remotes without removing the last remote. |

### Portable packs and migration

| Command | Responsibility |
| --- | --- |
| `orbit export [pack.zip]` | Export verified TOML, lock state, selected JARs, and portable configuration. JARs use ZIP Stored mode. |
| `orbit export pack.mrpack --format mrpack` | Export a Modrinth pack; remotely recoverable files stay indexed and local files become overrides. |
| `orbit import <file>` | Import TOML, safe ZIP content, or a Modrinth pack according to an explicit merge strategy. |
| `orbit migrate check <target>` | Resolve the complete graph against an already installed real target runtime. |
| `orbit migrate export <target>` | Reuse the same planner and write target TOML, lock, and configuration before `orbit install`. |
| `orbit migrate export <target> --source-pack source.zip --consume-source-pack` | Resolve from a source snapshot exported before target creation, then remove the snapshot after a confirmed export. |

The migration GUI sequence is intentionally transactional:

1. Export and validate the source Orbit pack.
2. Create and install the isolated target Minecraft/Loader runtime.
3. Run `orbit migrate check` and review the complete target package plan.
4. Confirm and export target Orbit state.
5. Run `orbit install` in the target; this creates `mods/` only when packages are actually materialized.

### Launcher

`orbit-launcher` is the runtime half of the suite. It creates isolated client instances and explicit
server directories from official Minecraft and Loader metadata; installs Vanilla, Fabric, Quilt,
Forge, and NeoForge; manages Mojang Java runtimes; handles Microsoft, offline, and standard
Yggdrasil accounts; and launches clients or supervises cancellable, restartable servers. It owns no
mod logic and never calls Orbit. The project Microsoft public-client registration is built in, while
tokens remain in the operating system's secret store.

An absent `mods/` directory is a valid empty mod set. Orbit does not manufacture it during init,
sync, checks, failed operations, or empty installs; it is created only when a selected JAR is
actually materialized. Loader-version updates remain a Launcher responsibility (`instance
configure --loader-version` followed by `install`), while cross-version migration creates a new
instance and lets Orbit check and migrate only the mod state.

```bash
# Install a complete isolated client in the default managed repository.
orbit-launcher install --new fabric-1.21.1 \
  --kind client --minecraft 1.21.1 --loader fabric

# Install a headless server into an explicit directory. The complete EULA must
# be shown and accepted through the dedicated command before it can run.
orbit-launcher install --new survival-server \
  --kind server --server-directory /srv/minecraft/survival \
  --minecraft latest-release --loader fabric
```

See the complete [Launcher CLI reference](docs/orbit-launcher-cli.md) and
[Launcher architecture](docs/orbit-launcher-architecture.md).

## State files

`orbit.toml` is the desired root-package configuration and remote set. `orbit.lock` is the exact selected graph, including content hashes, JAR metadata, environment declarations, dependencies, contained content, and materialization sources. Commit both files when you want an exact reproducible instance.

```toml
[project]
name = "survival"
mc_version = "1.20.1"
modloader = "fabric"
modloader_version = "0.15.7"

[platform]
minecraft_jar = { path = "../../1.20.1/1.20.1.jar", sha256 = "..." }
loader_jar = { path = "../../libraries/net/fabricmc/fabric-loader/0.15.7/fabric-loader-0.15.7.jar", sha256 = "..." }
runtime_jars = []
physical_environment = "client"

[dependencies]
sodium = { version = "^0.5", remotes = [
  { type = "modrinth", project_id = "AANobbMI" },
  { type = "curseforge", project_id = 394468 },
] }
```

The full schema is in [orbit.toml specification](docs/orbit-toml-spec.md).

## CurseForge API key

The CurseForge provider has no anonymous fallback. Configure a user-owned key before using `cf:`, a CurseForge catalog, or a CurseForge remote:

```bash
orbit config set auth.curseforge-api-key YOUR_API_KEY
```

Alternatively set `ORBIT_CURSEFORGE_API_KEY`. Secrets are redacted from configuration output and never enter manifests, lockfiles, progress events, or error bodies. See [provider documentation](docs/orbit-providers.md).

## Building

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

On Windows, `scripts/build-windows-msi.ps1` builds release binaries and the WiX 7 MSI. The GUI icon is generated from `assets/orbit.svg` during the Rust build and embedded in `orbit-gui.exe`; the SVG is the single source asset.

## Contributing

Issues and pull requests are welcome. Changes must preserve the crate boundaries, typed machine protocols, real Loader/JAR semantics, and ordered tests documented in [CLAUDE.md](CLAUDE.md).

## License

MIT License.
