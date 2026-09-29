# Phase 00 — Baseline and benchmarks

[Roadmap index](../README.md) · [Architecture overview](../architecture/overview.md) · [Bottlenecks](../architecture/bottlenecks.md) · [Phase 01](01-spatial-index-and-networking.md)

## Purpose

Establish a reproducible capacity envelope before modifying the network, world ownership, ECS, or plugin scheduler. The source baseline for this document set is Pumpkin upstream `master` at `4426d1113`. Later measurements must name the exact upstream-derived commit being tested; source inspection is not a substitute for benchmarks.

## Work to implement

1. **Reference workload manifest.** Record CPU model and physical cores, RAM, NIC, OS, Rust profile, commit, Java and Bedrock protocol versions, view and simulation distances, compression settings, entity mix, loaded chunks, plugin inventory, and admission settings. Record generator, client, and server hosts separately so load generators are not counted as server capacity.
2. **Server telemetry.** Add low-overhead spans/counters to the [ticker](../../crates/pumpkin/src/server/ticker.rs), [world phases](../../crates/pumpkin/src/world/mod.rs), [entity tracker](../../crates/pumpkin/src/world/entity_tracker.rs), [chunk generation](../../crates/pumpkin-world/src/chunk_system/schedule.rs), [Java decoder](../../crates/pumpkin-protocol/src/java/packet_decoder.rs), [Java outgoing writer](../../crates/pumpkin/src/net/java/outgoing.rs), and [plugin manager](../../crates/pumpkin/src/plugin/mod.rs). Include p50/p95/p99 tick time, phase CPU, queue length **and bytes**, output bytes, compression and cipher CPU, active entities, and process RSS.
3. **Correctness traces.** Capture deterministic seeds and tick-stamped inputs. Record state hashes or authoritative assertions for movement, collision, block updates, entity transfer cases, and plugin cancellation. Separate server correctness from packet-delivery timing.
4. **Controlled load profiles.** Include spread-out players, a 500-player 3×3-chunk event, tens of thousands of active entities, chunk churn, joins/disconnects, and mixed editions. Run with no plugins, representative plugins, and a deliberately expensive plugin. Hold arrival patterns and view distance constant across comparisons.
5. **Profiling.** Sample CPU, allocations, lock contention, cache misses, and network throughput. Identify whether each profile is limited by mandatory gameplay, scanning, compression, encryption, egress, plugin waits, or memory pressure.

## Workload matrix

| Profile | Main question | Minimum record |
| --- | --- | --- |
| Spread-out players | Does current world-level parallelism use available cores? | Players, dimensions, active chunks, region distribution proxy, per-world tick time |
| 500 in 3×3 chunks | What is the serial interaction and all-to-all replication cost? | Movers, observers, deliveries/s, bytes/s, tracking and collision CPU |
| Entity-dense world | Which entity types and systems dominate? | Active entities by type, AI and physics time, tracking candidates |
| Chunk churn | Do generation and lighting contend with play? | Generation backlog, section loads, light update time, tick p99 |
| Plugin fault | How far can a slow guest or host call affect the tick? | Event rate, decision wait, worker occupancy, trap or timeout behavior |
| Mixed Java/Bedrock | What edition-specific encoding and transport costs differ? | Edition mix, packet sizes, writer CPU, delivery latency |

The current Java `max_players` default is 1,000; changing it for a synthetic test is a test configuration, not a claim that the architecture scales. Never call a 10,000-socket login test a 10,000-active-player gameplay test.

## Completion gate

- A versioned workload manifest and raw results can reproduce p50/p95/p99 tick, input-to-commit, and commit-to-delivery latency on the named host.
- The benchmark reports payload and wire bytes, compression and cipher CPU, retained outbound bytes, RSS, and correctness failures separately.
- The hotspot profile quantifies `movers × recipients × update frequency`; its replication cost is visible rather than hidden in total CPU.
- Baseline runs and traces exist for strict gameplay and cancellable plugin behavior so later phases can detect semantic changes.

No 20 TPS or 10,000-player success is inferred from phase 00 alone. Its output is the reference against which [phase 01](01-spatial-index-and-networking.md) through [phase 05](05-capacity-validation.md) are judged.
