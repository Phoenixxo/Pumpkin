# Phase 05 - Capacity validation and release gate

[Roadmap index](../README.md) · [Overview](../architecture/overview.md) · [Bottlenecks](../architecture/bottlenecks.md) · [Phase 00](00-baseline-and-benchmarks.md) · [Phase 04](04-ecs-migration.md)

## Purpose

This phase determines the hardware, workload, and gameplay semantics under which one Pumpkin process can sustain a continuous world with 10,000 or more active players and tens of thousands of entities. The preceding architecture phases supply candidate mechanisms. A capacity claim follows only from this measured release gate.

## Prerequisites

The [phase 00 workload manifest](00-baseline-and-benchmarks.md) and baseline traces must be reproducible. [Spatial indexing and bounded egress](01-spatial-index-and-networking.md), [plugin execution controls](02-plugin-execution.md), [region ownership](03-region-ownership.md), and [hot entity storage](04-ecs-migration.md) must each have passed their correctness and latency gates. One server process owns the logical world; the setup identifies external load generators and monitoring services separately, with no hidden proxy-based world shard or client reconnect.

## Validation matrix

Each profile must run long enough to expose queue accumulation, retained memory, and periodic world work. Publish both warm-up and measurement duration so a brief peak cannot stand in for sustained capacity.

| Profile | Required conditions | Failure modes to exercise |
| --- | --- | --- |
| Small server, 1 to 100 active players | Replay the [phase 00](00-baseline-and-benchmarks.md) inputs with no plugins, v0.1 plugins, and a mixed ABI set in Global mode; compare Region mode where available. | Detect idle CPU, memory, tick, or plugin-latency regressions caused by extra task admission and routing. |
| 10,000+ spread-out active players | Players produce real gameplay inputs alongside moving entities, chunk subscriptions, and mixed protocol cohorts. | Exercise burst joins, disconnects, chunk churn, and slow readers. |
| 500-player 3×3-chunk crowd | Players are mutually visible and produce movement and interactions, with spectators or other observers where applicable. | Measure quadratic fanout, compression cohort misses, and hot-owner backlog. |
| Tens of thousands of entities | Publish the entity-type and AI/physics mix and keep entities within active simulation range. | Exercise pathfinding spikes, collisions, spawning and despawning, and tracker churn. |
| Border and migration stress | Repeat crossings, teleports, vehicle and mount transitions, and damage near owner boundaries. | Detect duplicate or lost entities, stale owner generations, and message cycles. |
| Plugin stress | Run v0.1 and v0.2 components together under both Global fallback and migrated Region ownership. Include nested same-Store and cross-Store calls, opposing causal roots, an infinite-loop guest, a host-call flood, waiter drop, and caller deadline expiry. | Detect legacy-gate unfairness, Store-owner stalls, reactor or Rayon starvation, reentry deadlock, accepted work lost on waiter drop, and late state commit. Check that fuel or epoch interruption contains CPU-bound guests. |
| Storage and generation | Continue chunk loading, generation, lighting, and saves during active play. | Detect background-pool starvation, stale snapshot publication, and I/O backlog. |

## Proposed acceptance measurements

These targets are proposed acceptance criteria, not measured current results. If a host cannot meet one of them, publish the narrower workload envelope that it does sustain.

- **Tick:** The p99 authoritative tick duration stays at or below 50 ms for the declared 20 TPS workload, reported per active region and for world/global coordination. Report the distribution and maximum as well as the average.
- **Delivery:** Publish p50, p95, and p99 input-to-commit and commit-to-client-send latency for nearby movement, block placement, and required state transitions. When movement is coalesced, also report the age of visible state.
- **Correctness:** Entity transfers are neither lost nor duplicated, and a stale command cannot commit under a new owner generation. Cancellation and collision match the declared compatibility contract, and deterministic replay reaches the expected authoritative state.
- **Bounded resources:** Ingress, plugin, background-job, per-client egress, and global egress byte limits remain effective. RSS and queue age remain bounded during a soak and a slow-reader attack.
- **Isolation:** A saturated hotspot or guest interrupted under the configured fuel or epoch policy stays within the published latency budget for unrelated regions. Required transactions follow their documented timeout policy; a caller deadline alone does not imply guest cancellation.
- **Protocol:** Java zlib and Bedrock transport interoperate with the targeted clients. Per-connection encryption and required-packet ordering remain correct when compatible frames are shared.

## Capacity accounting

Capacity results need the resource demand and delivered workload in the same record. For each profile, report physical cores and CPU utilization; memory and retained frame bytes; NIC line rate and payload/wire throughput; movers, observers, deliveries per second, and compressed-frame cache hit rate; active entities and chunks; background-job backlog; plugin decision and host-call rates by ABI lane; and managed-task queue age by domain. Identify whether the server or load generator set the observed limit. Report the fixed small-server profile as well as the capacity profiles so scheduler and routing overhead cannot hide behind dense-load gains.

The crowd calculation from the [overview](../architecture/overview.md) gives a planning lower bound: 500 movers visible to 499 others at 20 updates per second create 4.99 million deliveries per second. If the required protocol and visibility semantics demand more CPU or bandwidth than the host provides, report that limit. Coalescing visual updates may lower delivery count, but it cannot support a claim that every observer received every intermediate move.

## Publication format

A release statement should name the upstream-derived commit and include the complete workload manifest, duration, p99 tick and delivery results, correctness results, RSS, queue ceilings, CPU, and wire bandwidth. It should describe any gameplay or replication policy change from the baseline. The resulting claim applies to that measured configuration and workload envelope.
