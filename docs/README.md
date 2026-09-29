# Pumpkin continuous-world scaling blueprint

This blueprint describes how Pumpkin could run one continuous logical Minecraft world for more than 10,000 concurrent players and tens of thousands of active entities, without dividing the world behind BungeeCord or Velocity proxies. It is an implementation proposal and a plan for measuring that capacity, rather than a claim that the current server already supports it.

The account of existing behavior is based on [Pumpkin upstream `master` at `4426d1113`](https://github.com/Pumpkin-MC/Pumpkin/tree/4426d1113a211e6018a2db416e33b6b8a7802614), inspected on 2026-09-29. Throughout the documents, current code and work in progress are identified separately from proposed design. The [overview](architecture/overview.md) introduces the system contract and the reasons for the proposed boundaries.

## Architecture

| Document | Focus |
| --- | --- |
| [Overview](architecture/overview.md) | Establishes the capacity target, current implementation, data flow, and consistency contract. |
| [Spatial ownership and ECS](architecture/spatial-ownership-and-ecs.md) | Explains region ownership, Rayon jobs, entity layout, handoff, and crowded regions. |
| [Networking](architecture/networking.md) | Describes receive buffers, interest management, broadcast cohorts, compression, and backpressure. |
| [Plugin pipeline](architecture/plugin-pipeline.md) | Defines cancellable and observational events, WASM execution, host routing, and reentry. |
| [Bottlenecks and mitigations](architecture/bottlenecks.md) | Connects synchronization, memory, cache, fanout, and plugin risks to measurements. |

## Implementation phases

Each phase describes the change, its dependencies, how it can be introduced into the existing server, and the evidence required before proceeding. The numbers express dependency order; they do not promise a release schedule.

| Phase | Document |
| --- | --- |
| 00 | [Baseline and benchmarks](phases/00-baseline-and-benchmarks.md) |
| 01 | [Spatial index and networking](phases/01-spatial-index-and-networking.md) |
| 02 | [Plugin execution](phases/02-plugin-execution.md) |
| 03 | [Region ownership](phases/03-region-ownership.md) |
| 04 | [ECS migration](phases/04-ecs-migration.md) |
| 05 | [Capacity validation](phases/05-capacity-validation.md) |

## Reading convention

The word **current** refers to behavior present at the upstream commit named above. **In development** identifies a scaffold or partial implementation whose behavior is narrower than the eventual design. **Proposed** identifies a design in this blueprint that still requires implementation, compatibility work, and measurement.

The documentation branch is based on that upstream commit, so relative code links resolve against the inspected upstream source. Performance figures are arithmetic examples or proposed acceptance targets unless an actual measured result is explicitly identified.
