# Development guide

Orbit, Orbit Launcher and the desktop GUI are independent programs. This guide describes the
boundaries and invariants to preserve when changing them; the linked subsystem documents explain
the current implementation in more detail.

## Boundaries

- Keep CLI handlers thin: parse arguments, call core APIs, and render results. Business logic,
  file operations and dependency resolution belong in core. Core returns structured results and
  progress events; it does not print user-facing output.
- Provider wrappers own HTTP requests, DTOs, pagination and transport errors. Core adapts them to
  domain types. Use the shared HTTP retry layer for replayable requests, including body reads;
  do not automatically replay state-changing operations or stack retry policies.
- Launcher does not depend on Orbit. Orbit may wrap the existing Launcher command for runtime
  observation. The shared Launcher format crate contains only read-only, I/O-free contracts.
- GUI is a process client of the two CLIs, not another business API. It uses the selected or
  co-located executables and their strict UTF-8 machine protocol, without reading business files
  or falling back to a second implementation. Bind tasks and results to their explicit instance.
- Keep domain data and protocol keys language-neutral. Translate presentation at the CLI or GUI
  boundary; supported language selections are system, English and Simplified Chinese.

## Mod management

- JAR loader metadata determines package identity (`mod_id`), version, dependencies, environment
  and contained modules. Provider IDs and slugs locate artifacts, not package identities. Read
  metadata through the JAR module with the caller's explicit loader.
- Download and verify candidates before offline resolution. Preserve provider-specific data and
  provenance per artifact, use batch endpoints where available, and validate supplied hashes.
  CurseForge requires a user API key; never put it in artifacts, lockfiles, logs or error messages.
- Keep package content distinct from the physical top-level JAR used for installation or removal.
  Loader conditions, contained modules and the instance's physical environment belong in the
  shared dependency graph. The solver consumes real platform and lockfile metadata.
- `sync` records local facts without solving or deleting JARs; `install` materializes the exact
  lock without solving; `fix` repairs the graph. A constraint change is committed with its
  solution as one transaction. Preserve files and configuration on failure or cancellation.
- Use the shared portfolio and transaction paths for solving changes. Show consequential
  downgrades, replacements and removals before confirmation. Multiple identities or solutions
  require a user choice; `--yes` must not silently resolve ambiguity. Targeted and global
  upgrades have different objectives; see [the resolver guide](orbit-resolver.md).
- Migration plans against the actual target runtime and preserves source data. Export and install
  must use the same selected plan; do not silently retry with a different solution.

## Runtime and data safety

- Inject global paths through runtime context rather than discovering environment directories in
  business code. Keep provider indexes, JAR metadata databases and the global JAR cache separate.
- Use the manifest and lockfile abstractions for their file operations. Changes involving JARs and
  state must be transactional, with rollback or a clearly reported recovery path on failure.
- Runtime observation belongs to Orbit and records successful writes, not reads. Preserve shared,
  unknown and unobserved data; show the exact scope before destructive package-data operations.
- Do not expose secrets or internal content hashes in user-facing output. Keep diagnostic progress
  grounded in real typed events, not timers or parsed debug logs. Audit is a static risk analysis,
  not a guarantee of runtime compatibility.

## Changes and verification

Return actionable errors instead of stubs or false success. Prefer `Result` to unchecked failures
in libraries, and remove unused code introduced by a change. Test the affected behavior, including
loader-specific cases, cancellation, failure and rollback where relevant. For cross-program
contracts, test against the real producer and consumer rather than only separate fixtures.

Update the relevant command, architecture and subsystem documents when behavior changes. Useful
starting points are [architecture](orbit-architecture.md), [GUI](orbit-gui.md),
[Orbit commands](orbit-cli-commands.md), [Launcher commands](orbit-launcher-cli.md),
[output formats](orbit-output-formats.md) and [release process](release-process.md).

Common checks (GUI builds also need Node.js and Tauri system dependencies):

```sh
npm --prefix orbit-gui/ui ci
npm --prefix orbit-gui/ui test
npm --prefix orbit-gui/ui run build
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```
