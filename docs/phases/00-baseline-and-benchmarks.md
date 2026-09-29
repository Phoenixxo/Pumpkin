# Phase 00 - Baseline and benchmarks

[Roadmap index](../README.md) · [Architecture overview](../architecture/overview.md) · [Bottlenecks](../architecture/bottlenecks.md) · [Phase 01](01-spatial-index-and-networking.md)

## Purpose

The first phase establishes what Pumpkin can sustain today and where the time and memory go. Its source baseline is upstream `master` at `4426d1113`. Every later measurement should identify the exact upstream-derived commit under test. Source inspection explains how a path works; only a repeatable workload can establish its capacity.

## Measurement design

Begin with a versioned workload manifest. It should record the CPU model and physical core count, RAM, NIC, operating system, Rust build profile, commit, Java and Bedrock protocol versions, view and simulation distances, compression settings, entity mix, loaded chunks, plugin inventory, and admission settings. Record the generator, client, and server hosts separately so generator capacity cannot be mistaken for server capacity.

Instrument the [ticker](../../crates/pumpkin/src/server/ticker.rs), [world phases](../../crates/pumpkin/src/world/mod.rs), [entity tracker](../../crates/pumpkin/src/world/entity_tracker.rs), [chunk generation](../../crates/pumpkin-world/src/chunk_system/schedule.rs), [Java decoder](../../crates/pumpkin-protocol/src/java/packet_decoder.rs), [Java outgoing writer](../../crates/pumpkin/src/net/java/outgoing.rs), and [plugin manager](../../crates/pumpkin/src/plugin/mod.rs) with low-overhead spans and counters. The resulting trace should distinguish p50, p95, and p99 tick duration; CPU time by phase; queue length and **bytes**; output bytes; compression and cipher CPU; active entity counts; and process RSS. Sampling CPU, allocations, lock contention, cache misses, and network throughput then identifies whether mandatory gameplay, scanning, encoding, egress, plugin waits, or memory pressure sets the limit.

Correctness needs an equally stable reference. Capture deterministic seeds and tick-stamped inputs, then retain state hashes or authoritative assertions for movement, collision, block updates, entity transfers, and plugin cancellation. Measure packet-delivery timing separately from committed server state. Run the controlled profiles below with no plugins, representative plugins, and a deliberately expensive plugin; hold arrival patterns and view distance constant across comparisons.

## Workload matrix

| Profile | Main question | Minimum record |
| --- | --- | --- |
| Spread-out players | Does current world-level parallelism use the available cores? | Record players, dimensions, active chunks, a proxy for region distribution, and per-world tick time. |
| 500 in 3×3 chunks | What do serial interactions and all-to-all replication cost? | Record movers, observers, deliveries per second, bytes per second, and tracking and collision CPU. |
| Entity-dense world | Which entity types and systems dominate? | Record active entities by type, AI and physics time, and tracking candidates. |
| Chunk churn | Do generation and lighting contend with play? | Record the generation backlog, section loads, light update time, and tick p99. |
| Plugin fault | How far can a slow guest or host call affect the tick? | Record event rate, decision wait, worker occupancy, and trap or timeout behavior. |
| Mixed Java/Bedrock | How do edition-specific encoding and transport costs differ? | Record the edition mix, packet sizes, writer CPU, and delivery latency. |

The current Java `max_players` default is 1,000. Raising it for a synthetic test changes the test configuration; it does not establish that gameplay scales. A 10,000-socket login run and a 10,000-active-player gameplay run answer different questions and should be reported separately.

## Completion gate

- A versioned workload manifest and raw results reproduce p50, p95, and p99 tick, input-to-commit, and commit-to-delivery latency on the named host.
- The report separates payload and wire bytes, compression and cipher CPU, retained outbound bytes, RSS, and correctness failures.
- The hotspot profile quantifies `movers × recipients × update frequency` and shows replication cost separately from total CPU.
- Baseline runs and traces cover strict gameplay and cancellable plugin behavior so later phases can detect semantic changes.

Phase 00 produces the reference against which [phase 01](01-spatial-index-and-networking.md) through [phase 05](05-capacity-validation.md) are judged. It makes no 20 TPS or 10,000-player claim on its own.
