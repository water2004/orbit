# curseforge-wrapper

Typed HTTP access to the CurseForge API.

This crate owns the CurseForge transport boundary: request parameters, response
models, authentication headers, pagination, status handling, and HTTP retries.
It does not contain Orbit package identity, dependency solving, cache, or
installation logic.

Creating a client requires a CurseForge API key:

```rust
let client = curseforge_wrapper::Client::new(api_key, "orbit/0.1")?;
let games = client.games().await?;
```
