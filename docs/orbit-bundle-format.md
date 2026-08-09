# Orbit Bundle Format

`.orbitbundle` is Orbit's native, versioned package format. It is a ZIP
container whose root contains exactly one `bundle.toml` plus the files listed
by that manifest. The filename extension is part of the CLI contract; generic
`.zip` files are deliberately not accepted.

## Ownership model

One package may contain two independent optional projections:

- `launcher`: reproducible Minecraft/Loader/Java requirements and, when
  requested, mutable game state such as worlds and preferences;
- `orbit`: package constraints, `orbit.toml`, `orbit.lock`, selected JARs and,
  when requested, runtime-owned data/configuration.

Every payload file has one owner (`launcher` or `orbit`) and MUST be
stored below the corresponding top-level directory. Launcher and Orbit may
read the common identity, runtime and targets. Each program may only interpret
or mutate its own projection. Composition copies the other projection as
opaque, hash-verified payload.

This is one archive format, not two archives joined by convention. A valid
package MUST contain at least one Launcher or Orbit section.

## Root manifest

```toml
format-version = 1
id = "example-pack"
name = "Example Pack"
version = "1.0.0"
targets = ["client"]

[runtime]
minecraft = "1.21.1"
loader = "fabric"
loader-version = "0.16.14"

[launcher]
content = "runtime-and-state" # or runtime-only

[orbit]
content = "mods-and-data"    # or mods
manifest = "orbit/orbit.toml"
lock = "orbit/orbit.lock"
ownership = "orbit/.orbit/runtime-data/ownership.toml"
data-manifest = "orbit/.orbit/portable-data.toml"

[[files]]
path = "orbit/mods/example.jar"
owner = "orbit"
size = 1234
sha256 = "...64 lowercase hex characters..."
```

For `orbit.content = "mods"`, `ownership` and `data-manifest` MUST be absent.
For `mods-and-data`, both are required and must refer to declared files.
Vanilla runtimes omit `loader-version`; modded runtimes require an exact
Loader version.

## Validation and transactions

Consumers MUST reject the package before mutation when any structural rule is
violated or any file in the projection they consume fails integrity:

- `bundle.toml` is missing, too large, malformed, or uses an unsupported
  format version;
- a path is absolute, contains `..`, `.`, backslashes, an empty component, or
  is outside its declared owner namespace;
- the ZIP inventory differs from the manifest, contains duplicates, or
  contains a symbolic link;
- a consumed file's declared size or SHA-256 differs from its actual bytes;
- required runtime, target, section, or control-file invariants do
  not hold.

Owner extraction verifies only that owner's files, so a large Launcher world
does not get read by Orbit and a large mod set does not get read by Launcher.
An operation that preserves or composes an opaque projection verifies the
whole input package before copying it. Extraction is staged first. Target
mutation is one rollback-capable file transaction. Orbit native import keeps the target instance's exact detected
platform paths while importing package state. Launcher state restoration
keeps the target's generated `server.properties` schema and transfers only
keys that still exist; EULA acceptance is never packaged.

## CLI composition

```text
# Orbit projection only; choose whether package-owned data is included.
orbit export pack.orbitbundle --format orbit --content mods-and-data

# Append Launcher state to the same package atomically.
orbit-launcher --instance source export pack.orbitbundle \
  --base pack.orbitbundle

# A new runtime can use the common requirement. Explicit runtime arguments
# intentionally override native-bundle requirements for cross-version
# migration; mrpack dependencies remain exact.
orbit-launcher install --new target --kind client \
  --minecraft 1.21.2 --loader fabric --from pack.orbitbundle
```

The GUI uses this exact CLI path. It does not parse or modify packages itself.

## Modrinth `.mrpack`

`.mrpack` is not an alias for `.orbitbundle`. Orbit implements Modrinth
[`formatVersion = 1`](https://support.modrinth.com/en/articles/8802351-modrinth-modpack-format-mrpack)
directly:

- `game` must be `minecraft`;
- dependencies use `minecraft`, `fabric-loader`, `quilt-loader`, `forge`, or
  `neoforge`;
- indexed files require SHA-1, SHA-512, size, HTTPS downloads, and official
  client/server `required|optional|unsupported` environment values;
- `overrides/` is applied first, followed by the selected
  `client-overrides/` or `server-overrides/` layer;
- optional indexed files are installed only when explicitly selected by path
  (or when the caller explicitly selects all optional files for that side);
- Orbit metadata is never written into mrpack overrides.

Launcher consumes only the official runtime dependency map. Orbit consumes
indexed files and overrides. This division does not change the mrpack schema.
`orbit-launcher package inspect` exposes the common/runtime view, allowing a
frontend to choose a side and optional files before coordinating the two CLIs.
