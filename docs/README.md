# Pumpkin continuous-world scaling blueprint

**Status:** architecture and implementation proposal, not a capacity claim or an implemented feature set. The current-state baseline is [Pumpkin upstream `master` at `4426d1113`](https://github.com/Pumpkin-MC/Pumpkin/tree/4426d1113a211e6018a2db416e33b6b8a7802614), inspected on 2026-09-29. Proposed work is labeled separately from upstream code.

The target is one continuous logical Minecraft world, more than 10,000 concurrent players, and tens of thousands of active entities without a BungeeCord/Velocity-style proxy split. Capacity must be demonstrated on specified hardware and workload settings. The [overview](architecture/overview.md) gives the system contract and the boundary between current code and proposed work.

## Architecture

| Document | Focus |
| --- | --- |
| [Overview](architecture/overview.md) | Goals, current state, system flow, consistency rules, capacity limits |
| [Spatial ownership and ECS](architecture/spatial-ownership-and-ecs.md) | Region actors, Rayon jobs, entity layout, handoff, crowded regions |
| [Networking](architecture/networking.md) | Receive buffers, interest management, broadcast cohorts, compression, backpressure |
| [Plugin pipeline](architecture/plugin-pipeline.md) | Cancellable versus observational events, WASM execution, host routing, reentry |
| [Bottlenecks and mitigations](architecture/bottlenecks.md) | Synchronization, memory, cache, fanout, and plugin risks with measurements |

## Implementation phases

Each phase has its own scope, dependencies, migration approach, and completion gate. The numbers express dependency order, not a release promise.

| Phase | Document |
| --- | --- |
| 00 | [Baseline and benchmarks](phases/00-baseline-and-benchmarks.md) |
| 01 | [Spatial index and networking](phases/01-spatial-index-and-networking.md) |
| 02 | [Plugin execution](phases/02-plugin-execution.md) |
| 03 | [Region ownership](phases/03-region-ownership.md) |
| 04 | [ECS migration](phases/04-ecs-migration.md) |
| 05 | [Capacity validation](phases/05-capacity-validation.md) |

## Reading convention

- **Current** means present on the upstream commit named above.
- **In development** means an upstream scaffold or partial implementation whose documented behavior is narrower than the full design.
- **Proposed** means this blueprint's design and requires implementation, compatibility work, and measurement.

The documentation branch is based on that upstream commit, so relative code links resolve against the inspected upstream source. Performance figures are arithmetic examples or proposed acceptance targets unless an actual measured result is explicitly identified.
