# Pareto update solver regression (2026-09-22)

This is a measured regression case, not a general complexity or speedup claim.

## Input and procedure

- User-provided Minecraft 26.1.2 collection: 94 JARs.
- ZIP SHA-256: `e11648abe7697126fac4020a874786f27a6bef5ddc4400ee0ed1a27213786582`.
- Separate disposable client workspace, Fabric 0.19.3; no constraints inherited
  from the user's existing mod list. `sync` identifies the supplied JARs first.
- Warm version repository/JAR cache; both implementations received 107 logical
  candidate packages and 563 physical candidates after discovery.
- Baseline: Orbit `e9389ff`, PubGrub `6c4fd57`.
- New solver: PubGrub `fb5b61e`, with Orbit's finite-domain package priority and
  updated observer integration.
- Release builds on the same Windows host, `orbit outdated --yes
  --progress-format ndjson --output-format json`. Only the disposable workspace
  is used. This command is read-only; the harness answers each structured
  selection with its first alternative to exercise final composition as well.
- Timings distinguish preparation from the interval between `resolution_started`
  and the first selection request. Peak memory is process peak working set,
  not a heap-only measurement. These are single-run observations.

## Results

| Measurement | Baseline | New implementation |
| --- | ---: | ---: |
| Preparation | 8.0 s | 7.3 s |
| Solver to first choice | Still searching after 82.1 s; stopped | 22.0 s |
| Whole command | Stopped at 90.1 s | 29.6 s, successful result |
| Process CPU time | 56.8 s before stopping | 22.6 s total |
| Peak working set | 1,055 MiB before stopping | 316 MiB |
| Propagations | 3,202,715 before stopping | 396,499 total |
| Conflicts | 24,432 before stopping | 2,205 total |

The new run presents 15 non-singleton groups with respectively
`72, 3, 2, 40, 2, 2, 9, 2, 2, 10, 2, 3, 4, 2, 2` alternatives.
Their product is 2,388,787,200 complete combinations; these combinations are
not materialized. Final composition and dependency verification succeed.
This does not claim the groups are the smallest mathematically possible factors.

A subsequent run built from the pinned Git dependency (without a local Cargo
patch) completed successfully in 26.2 s total, including 17.8 s to generate the
choices, with a 316 MiB peak working set. Timings vary between runs; the table
retains the first complete measurement rather than presenting only the fastest.

The baseline's zero retained `Solution` events does **not** mean no feasible
assignment was found: it was still proving front invariants. Decision and
propagation counts measure search work, not the number of output plans.

## Regression protection

The fork tests include all nonempty 3-by-3 feasible relations against a brute-force
Pareto oracle, optional-presence products, equal-rank realizations, coordinated
upgrades, and an ascent deliberately starting at the lowest versions. A separate
exhaustive test verifies that learned substitution regions contain no Pareto
points, including hidden variables and optional projected packages. A 4,096-state
optional product is checked with a bound on solver continuation count rather than
a wall-clock timeout.

Orbit tests check incremental observer checkpoint/rollback behavior. CLI and GUI
share the new stage vocabulary; neither layer implements a second solver.
No candidate limit, approximate Pareto policy, loader exception, or per-mod
special case is used to obtain these timings.
