<p align="center">
  <img src="assets/orbit.svg" width="112" height="112" alt="Orbit logo">
</p>

# Orbit

[简体中文](README.zh-CN.md)

**A modern, non-intrusive package manager and native workspace for Minecraft Java Edition.**

Orbit brings package-manager semantics to Minecraft mods without replacing the launcher workflow you already use. It can adopt an existing valid game instance, reconstruct its package graph from JAR-declared metadata, discover candidates from multiple providers, resolve complete dependency portfolios, and restore an exact lockfile.

The repository contains three deliberately separated applications:

- `orbit` manages mods and their dependency graph.
- `orbit-launcher` manages Minecraft, Loaders, Java, accounts, launching, and supervised servers. It never links to or calls Orbit; Orbit can wrap its start command when package-owned runtime data observation is requested.
- `orbit-gui` is a native GPUI shell. It performs no package or launcher business logic. Runtime installation/accounts still call Launcher directly, while game start and package-data purge go through Orbit's joint-launch path.

The detailed boundaries are documented in [Orbit architecture](docs/orbit-architecture.md), [Launcher architecture](docs/orbit-launcher-architecture.md), [GUI architecture](docs/orbit-gui.md), and the [native bundle format](docs/orbit-bundle-format.md).

## Highlights

- **Existing instances and isolated managed instances.** Orbit works in valid Fabric, Quilt, Forge, and NeoForge client or dedicated-server game directories. Launcher-managed clients use isolated `instances/<instance>` game directories with an instance-owned `minecraft.jar` and shared immutable assets/libraries; Launcher-managed dedicated servers keep their complete locked runtime in the selected server directory.
- **Truthful synchronization.** `orbit sync` redetects the runtime, scans local JARs, and uses provider hash APIs to recover sources. It makes TOML, lock, and package-group state exactly match the local JAR set without solving dependencies or deleting JAR files.
- **Explicit repair.** `orbit fix` is the command that discovers the full recursive candidate closure, resolves it, presents every package-level action, and applies a confirmed repair.
- **Multiple remotes per package.** A logical package can use local files, Modrinth projects, and CurseForge projects together. Content is deduplicated by hash; identity, version, dependencies, environment, `provides`, and bundled content come only from downloaded JAR metadata.
- **Scoped local version repository.** Each exact Minecraft/Loader pair has separate provider-snapshot and JAR-analysis databases. Provider project markers are checked in batches; unchanged projects reuse local versions, while changed projects refresh only the active game version. The global LRU JAR cache remains a separate content store.
- **Loader-faithful metadata and audit backends.** Fabric, Quilt, Forge, and NeoForge keep their real registration, nesting, namespace, and Mixin activation rules before entering shared normalized models.
- **Explainable objective-aware solving.** Dependency causes come from the actual PubGrub propagation and backtracking path. `add` and `fix` enumerate standard Pareto-minimal package-change sets; `upgrade` and `outdated` enumerate the standard Pareto-maximal version front. Every incomparable alternative remains an explicit choice.
- **Package-level transactions.** `mod_id` is the solver package key. A selected package may own multiple contained JARs, while unselected top-level package versions are removed only after the exact plan is shown and confirmed.
- **Observable and cancellable work.** Discovery, downloads, solving, application, audit, and package export emit typed progress. Already-compressed JARs are stored directly, real byte progress is reported, and failed temporary output is removed.
- **One package, disjoint owners.** `.orbitbundle` is a versioned, hash-inventoried format with optional Launcher and Orbit sections. Orbit owns mods, constraints, lock state, and opted-in package data; Launcher owns runtime requirements, worlds, and game preferences. The GUI composes both projections into one migration bundle without either CLI interpreting the other's files. Official `.mrpack` import/export remains a separate strict implementation of Modrinth's format.
- **Runtime-owned package data.** `orbit launch` wraps Orbit Launcher and injects Orbit's low-overhead Java Agent. It never observes reads. A verified, version-ranged Loader capability table maps physical JARs, Forge-family union sources, and Quilt native module identities back to one top-level logical package. A file belongs to its last successful package writer; the first package writing into an unowned directory claims that directory, while shared game/world roots remain unowned and specific descendants may still be owned. When a declared library dependency performs I/O for its caller, the Agent attributes the write through that dependency edge instead of hard-coding library or mod names. Growing exceptions are recompressed from metadata on the cold merge path so directory defaults follow the owner of the most actual files without changing per-file purge or migration results. Unknown Loader/JVM ranges reject observation instead of guessing; filenames and static analysis are never used as ownership evidence.

## Installation

Windows x64 users can install the complete suite from `orbit-<version>-x86_64.msi` on the GitHub Releases page. The installer provides three feature profiles: Orbit only, Orbit + Launcher, or the complete suite with the native GUI. It can add the CLI directory to system `PATH`, supports same-version maintenance, and asks whether default AppData configuration and caches should be removed during uninstall.

Debian and Ubuntu amd64 releases contain three independently installable packages. A headless
server needs only Launcher, and can add Orbit when it also wants managed mods:

```bash
# Headless Minecraft runtime management.
sudo apt install ./orbit-launcher_0.5.0-1_amd64.deb

# Optional mod package management on that server.
sudo apt install ./orbit_0.5.0-1_amd64.deb

# Desktop installation: apt resolves the GUI's exact-version CLI dependencies
# when all three downloaded files are supplied together.
sudo apt install ./orbit_0.5.0-1_amd64.deb \
  ./orbit-launcher_0.5.0-1_amd64.deb \
  ./orbit-gui_0.5.0-1_amd64.deb
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
# Constraints are explicit CLI input; this example excludes two author-defined labels.
orbit add lithium --string 'all; intersect not contains(i"beta"); intersect not contains(i"snapshot")'
orbit add cf:238222
orbit add file:./my-local-mod.jar

# Manage all candidate remotes of one logical package.
orbit remote add sodium modrinth AANobbMI
orbit remote add sodium curseforge 394468
orbit remote list sodium

# Optional package environment filtering; auto follows the selected JAR.
orbit env sodium client
orbit env sodium auto

# Inspect JAR-declared versions from every configured remote, then apply a policy.
orbit versions sodium
orbit constraint set sodium exact 0.6.13
# Ordered rules inspect the complete JAR-declared version string.
orbit constraint set sodium any --string 'all; intersect not contains(i"beta")'
# Other structured forms: any, greater-than, at-least, less-than, at-most,
# or: range <lower> <upper> [--lower-bound ...] [--upper-bound ...]

# Repair, restore, update, and audit.
orbit fix
orbit install
orbit outdated
orbit upgrade
orbit audit
```

Both CLIs accept `--language system|en|zh-CN`. Orbit's persisted `core.language` defaults to `system`, and an explicit flag overrides it. Human-readable help, output, progress, prompts, and errors follow the selected language. JSON, NDJSON, stdin responses, schema fields, enum codes, and error codes remain strict UTF-8 and language-neutral.

## Command model

Orbit resolves its instance from the current directory first, then an explicit `--instance`, then the read-only global default.

### Instance and package state

| Command | Responsibility |
| --- | --- |
| `orbit init <name>` | Initialize a valid game directory from redetected local facts. |
| `orbit sync` | Redetect the runtime, scan JARs, recover known remotes online, and make TOML/lock/group facts exactly match local JARs without dependency solving or deleting JAR files. |
| `orbit fix` | Find and confirm a standard Pareto-minimal package repair; this is the repair command. |
| `orbit install [--target client\|server]` | Materialize the exact existing lockfile. It does not solve, repair, remove packages, or rewrite state. |
| `orbit add <locator>` | Add a provider project or local file while Pareto-minimizing changes to the existing instance. |
| `orbit enable/disable <package>` | Toggle Loader discovery by atomically renaming the selected JAR and recording the package state. |
| `orbit remove <package>` | Remove the logical package and its TOML/lock state. |
| `orbit launch [--server]` | Start the current client or server through Orbit Launcher while recording runtime data ownership. Both Orbit and Launcher must be installed. |
| `orbit purge <package>` | Show one exact package/data deletion plan, including preserved nested current owners, then remove the logical package from TOML/lock and recursively delete its observed ownership roots. |
| `orbit reset <package>` | Show the same exact data-only plan, then transactionally remove the package's observed data while preserving its JAR, TOML, lock, remotes, and constraints. |
| `orbit outdated [package]` | Explain feasible updates and why newer candidates are blocked. |
| `orbit upgrade [package]` | Apply a solution in which at least one requested package becomes newer; dependencies may downgrade or be replaced. |
| `orbit list [--tree]` | Show installed logical packages from the lockfile. |
| `orbit remote add/remove/list` | Manage local, Modrinth, and CurseForge remotes without removing the last remote. |
| `orbit versions <package>` | Refresh changed projects for the current Minecraft/Loader scope, reuse locally analyzed content, then list JAR-declared versions in descending order. |
| `orbit constraint show/set` | Inspect or atomically apply a numeric-core policy plus an ordered complete-string rule using a Pareto-minimal package transaction. |

### Portable packs and migration

| Command | Responsibility |
| --- | --- |
| `orbit export [pack.orbitbundle] [--content mods\|mods-and-data]` | Export a verified Orbit projection. The manifest inventories every file by owner, size, and SHA-256; JARs use ZIP Stored mode. |
| `orbit export pack.mrpack --format mrpack [--content mods\|mods-and-data]` | Export an official Modrinth pack; recoverable files stay indexed, local JARs become overrides, and selected state uses the official side-specific override layer. |
| `orbit import <file>` | Import TOML, a validated `.orbitbundle`, or an official `.mrpack`. Optional mrpack paths are selected explicitly with repeatable `--optional` (or `--all-optional`); generic ZIP files are not accepted. Import does not secretly run `fix`. |
| `orbit migrate check <target>` | Resolve the complete source package set against an installed target runtime; if that set is impossible, offer a Pareto-minimal package-removal search. |
| `orbit migrate export <target>` | Reuse the same strict-first planner and write target TOML, lock, and configuration before `orbit install`. |
| `orbit migrate export <target> --source-pack source.orbitbundle --consume-source-pack` | Resolve from a source bundle exported before target creation, then remove it after a confirmed export. |

The migration GUI sequence is intentionally transactional:

1. Export and validate the source Orbit projection, then append Launcher-owned state to the same bundle.
2. Create and install the isolated target Minecraft/Loader runtime.
3. Run `orbit migrate check` and review the complete target package plan.
4. Confirm and export target Orbit state.
5. Register the validated target state in Orbit's global instance list.
6. Run `orbit install` in the target; this creates `mods/` only when packages are actually materialized.

Migration has no up-front retention strategy selector. Orbit first requires every source package.
Only when that graph is unsatisfiable does the same CLI process show the incompatibility and ask
whether to search the standard Pareto-minimal package-removal front. Automation may provide the same
consent with `--allow-removals`. Independent removal trade-offs are returned as a factored product:
the CLI asks once per factor instead of expanding every Cartesian combination, then enumerates the
version Pareto front only for the selected removal assignment. Every remaining incomparable choice
still requires an explicit decision.

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

Launcher state is exported as the Launcher projection of the same package format and restored only through installation. `--base` atomically composes it with an Orbit projection:

```text
orbit export migration.orbitbundle --format orbit --content mods-and-data
orbit-launcher --instance old-client export migration.orbitbundle --base migration.orbitbundle
orbit-launcher install --new new-client --kind client \
  --minecraft 1.21.1 --loader fabric --loader-version stable \
  --from migration.orbitbundle
```

Client saves come from the isolated instance `saves/` directory. Dedicated-server worlds follow
`server.properties` `level-name` (default `world`); target Minecraft generates its own property
schema and Launcher migrates only values for fields that still exist. EULA acceptance is never
migrated.

See the complete [Launcher CLI reference](docs/orbit-launcher-cli.md) and
[Launcher architecture](docs/orbit-launcher-architecture.md).

## State files

`orbit.toml` is the complete managed logical-package set and its policies. Every selected top-level package has an equal `[packages.<mod_id>]` entry; there is no root/transitive distinction. `orbit.lock` records only the exact selected facts, including content hashes, JAR metadata, dependencies, contained content, and materialization sources. Commit both files when you want an exact reproducible instance.

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

[packages]
sodium = { version = "^0.5", string = 'all; intersect not contains(i"beta")', remotes = [
  { type = "modrinth", project_id = "AANobbMI" },
  { type = "curseforge", project_id = 394468 },
] }
```

`version` is only a numeric-core rule. `=1.2.3` therefore matches every
Loader-valid representation whose numeric core is `1.2.3`; author text such as
`-alpha` belongs in `string`, not in a numeric operand. Representations remain
distinct solver choices but have equal upgrade/Pareto precedence when their
numeric cores are equal.

The optional `string` rule sees the complete JAR-declared version, including
any prefix, numeric text, separators, qualifiers, and build text. It starts
from `all` or `none`, then applies ordered `intersect [not]`, `union [not]`, and
whole-set `complement` operations. Quoted strings are exact and case-sensitive;
`i"text"` ignores case. Orbit assigns no release-stage meaning to author text.
The CLI never inserts an implicit constraint: callers pass the complete rule with
`orbit add --string`. The GUI offers a default-checked recommendation that passes the rule
excluding case-insensitive `beta` and `snapshot` directly to that same option; unchecking it
omits the option. Existing entries are never rewritten by a frontend recommendation.
Numeric cores may have any number of components. A Fabric/Quilt opaque version
bypasses only `version` and still goes through `string`, with a visible warning;
Forge/NeoForge retain their Loader rule that declared mod versions must start
with a digit.

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
