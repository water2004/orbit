# Installation versus Loader activation: verification

## Scope

Baseline: `5813d2a`, before the installation/activation split. Same machine and
Rust debug profile; timings exclude compilation and network. Ten runs per case,
median elapsed time. Working set is sampled high-water mark during the process,
not an allocator-only measurement. These are representative regressions, not a
bound on every real mod set.

The reusable offline harness is `orbit-core/examples/profile_solver.rs`:

```text
cargo run -p orbit-core --example profile_solver -- 32 10 max
cargo run -p orbit-core --example profile_solver -- 32 10 min
```

32 packages, 96 candidate JAR metadata records. Each package has two different
suffixes at the highest numeric version: the maximum-version front represents
2^32 (4,294,967,296) complete assignments. The minimum-change baseline has all
32 packages installed at the earlier version.

| Objective | Before median | After median | Before working set | After working set |
| --- | ---: | ---: | ---: | ---: |
| Maximum versions | 105.37 ms | 111.47 ms | 15.08 MiB | 15.23 MiB |
| Minimum changes | 68.29 ms | 73.47 ms | 13.37 MiB | 13.96 MiB |

Maximum-version observer counters are unchanged: 2,243 decisions, 1,379
propagations, 128 conflicts, 32 backtracks, 65 solver solutions. Minimum-change
counters are also unchanged: 70 decisions, 167 propagations, two conflicts,
zero backtracks, two solutions. No complete Cartesian product is materialized.

The initial uncoalesced model doubled most ordinary decision/propagation work.
Portfolio compilation now coalesces only proven bijective installation/module
bindings, including absence, and maps every dependency and clause. Mixed-side
candidate sets and competing contained providers remain separate. Exhaustive
side/version fixture combinations compare feasibility before/after coalescing;
existing factor, optional-presence, upgrade and migration regressions remain.
The remaining measured cost is graph construction/analysis, not zero overhead.

## Fabric oracle

An isolated temporary dedicated server used Minecraft 1.21.1, Fabric Loader
0.19.2, and the real Modrinth MaLiLib 0.21.10 JAR (`GcWjdA9I` / `C99LEy6r`).
Its SHA-512 was checked against Modrinth. A metadata-only server-compatible
consumer declares `malilib >=0.21`.

- With MaLiLib installed, Fabric passed dependency resolution, loaded the
  consumer, and did not list MaLiLib among loaded modules.
- Renaming MaLiLib to `.jar.disabled` caused Fabric to fail with
  `HARD_DEP_NO_CANDIDATE` and a request to install MaLiLib >=0.21.
- Orbit init rejected that disabled state. After `package malilib enable`,
  `orbit install` validated both exact packages, retained the library, and
  `list` reported its `environment_disabled` load status.

The first server run subsequently failed during the local Java NIO socket
initialization (`Unable to establish loopback connection`), after Loader
resolution. This verifies the Loader boundary, not a complete gameplay/server
startup pass. No user instance, installed GUI or Minecraft client was started
or modified. Test configuration, registry and caches were isolated.
