<p align="center">
  <img src="assets/orbit.svg" width="96" height="96" alt="Orbit logo">
</p>

# Orbit

[简体中文](README.zh-CN.md) · [Downloads](https://github.com/water2004/orbit/releases) · [Documentation](docs/orbit-cli-commands.md)

Orbit manages Minecraft mods in an existing game directory without requiring you to switch
launchers. Orbit Launcher can also create and run Minecraft instances, and Orbit GUI brings both
tools together in a desktop interface. The command-line tools can be used independently.

![Orbit instance library](assets/screenshots/instance-library.png)

![Minecraft and Loader selection](assets/screenshots/create-instance.png)

The screenshots show the Chinese interface with demonstration data. English and system language
selection are also available.

## What you can do

Orbit reads mod identities and dependencies from JAR metadata. It can find mods on Modrinth and
CurseForge, manage version rules, check for updates, repair dependencies and restore a locked
set of files. Its compatibility audit reports potential bytecode conflicts; it cannot guarantee
that a modpack will run correctly.

Launcher installs Minecraft with Fabric, Quilt, Forge or NeoForge where supported, manages Java
and accounts, and runs clients or supervised dedicated servers. It can also preview and import
supported instances from other launchers into a new isolated instance. Import copies the source
files and leaves the original in place; see [import support](docs/orbit-launcher-import.md) for
the supported layouts and limitations.

The desktop app provides an instance library, mod discovery, installation, accounts, version
rules, pack import/export and migration. It supports Orbit bundles and Modrinth `.mrpack` files.

## Install

Download a package from [GitHub Releases](https://github.com/water2004/orbit/releases).

- **Windows x64:** the unsigned MSI offers Orbit only, Orbit with Launcher, or the complete suite.
  The GUI needs WebView2.
- **Debian/Ubuntu amd64:** install the separate `orbit`, `orbit-launcher` and `orbit-gui` deb
  packages as needed. The GUI requires matching versions of both CLIs and WebKitGTK 4.1.

See the [Windows](docs/windows-msi.md) and [Linux](docs/linux-deb.md) installation guides for
details. In the GUI, Settings → About can check GitHub for a newer stable release and open its
release page; it does not install updates automatically.

## Get started

To manage mods in an existing supported game directory:

```sh
cd /path/to/your/game-directory
orbit init survival
orbit sync
orbit add sodium
```

Use `orbit sync` to reconcile local files, `orbit fix` to solve dependency problems, and
`orbit install` to restore the exact lockfile. `orbit outdated` shows available updates and
`orbit upgrade` applies them after review. See the [Orbit command reference](docs/orbit-cli-commands.md)
for more options.

To create a new isolated client with Launcher:

```sh
orbit-launcher install --new survival --kind client \
  --minecraft latest-release --loader fabric --loader-version stable
```

The GUI offers a guided alternative. For existing instances and server setup, see the
[Launcher command reference](docs/orbit-launcher-cli.md).

CurseForge access requires your own API key. Configure it with
`orbit config set auth.curseforge-api-key YOUR_API_KEY` or the
`ORBIT_CURSEFORGE_API_KEY` environment variable.

## Documentation and development

- [Orbit commands](docs/orbit-cli-commands.md) · [Launcher commands](docs/orbit-launcher-cli.md)
- [Manifest schema](docs/orbit-toml-spec.md) · [Version rules](docs/orbit-versions.md)
- [Portable packs](docs/orbit-bundle-format.md) · [Compatibility audit](docs/orbit-bytecode-audit.md)
- [GUI architecture and build](docs/orbit-gui.md) · [Development guide](docs/development.md)

## License

Orbit and Orbit Launcher are MIT. The GUI adapts Modrinth App components and is GPL-3.0-only;
see the [GUI license](orbit-gui/ui/LICENSE), [upstream attribution](orbit-gui/ui/UPSTREAM.md),
and [screenshot provenance](assets/screenshots/README.md).
