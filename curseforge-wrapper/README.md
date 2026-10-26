# curseforge-wrapper

Typed HTTP access to the CurseForge API.

This crate owns the CurseForge transport boundary: request parameters, response
models, authentication headers, pagination, status handling, and HTTP retries.
It does not contain Orbit package identity, dependency solving, cache, or
installation logic.

`FileIndex.mod_loader` is optional: live Minecraft project responses can omit
`modLoader` on historical entries (verified with IMBlocker, project 483760).
Absent/null means unknown; it must not be converted to the explicit `0 = Any`
enum value. Search filtering belongs to the API's `gameVersion` /
`modLoaderType` parameters, not a completeness assumption about latest-file
summaries. See the [official FileIndex schema](https://docs.curseforge.com/rest-api/#tocS_FileIndex).

Creating a client requires a CurseForge API key:

```rust
let client = curseforge_wrapper::Client::new(api_key, "orbit/0.1")?;
let games = client.games().await?;
```
