# Phase 05 — Capacity validation and release gate

[Roadmap index](../README.md) · [Overview](../architecture/overview.md) · [Bottlenecks](../architecture/bottlenecks.md) · [Phase 00](00-baseline-and-benchmarks.md) · [Phase 04](04-ecs-migration.md)

## Purpose

Establish the exact workload envelope, hardware, and semantics for which Pumpkin can sustain a continuous world with 10,000+ active players and tens of thousands of entities. This phase is a proof gate, not a presumption that the previous architecture work guarantees the number.

## Prerequisites

- The [phase 00 workload manifest](00-baseline-and-benchmarks.md) and baseline traces are reproducible.
- [Spatial indexing and bounded egress](01-spatial-index-and-networking.md), [plugin execution controls](02-plugin-execution.md), [region ownership](03-region-ownership.md), and [hot entity storage](04-ecs-migration.md) have passed their own correctness and latency gates.
- A single server process owns the logical world. Any external load generators and monitoring services are declared separately. No proxy-based world shard or client reconnect is hidden in the setup.

## Validation matrix

Run each profile long enough to expose queue accumulation, memory retention, and periodic world work. Publish duration and warm-up period rather than reporting only a short peak.

| Profile | Required conditions | Failure modes to exercise |
| --- | --- | --- |
| 10,000+ spread-out active players | Actual gameplay inputs, moving entities, chunk subscriptions, mixed protocol cohorts | Burst joins, disconnects, chunk churn, slow readers |
| 500-player 3×3-chunk crowd | Mutually visible players, movement and interaction mix, spectators or additional observers where applicable | O(N²) fanout, compression cohort misses, hot owner backlog |
| Tens of thousands of entities | Published entity-type and AI/physics mix, active simulation range | Pathfinding spikes, collisions, spawning/despawning, tracker churn |
| Border and migration stress | Repeated crossings, teleports, vehicles, mounts, damage near owner boundaries | Duplicate/lost entities, stale owner generations, message cycles |
| Plugin stress | Typical v0.1 and v0.2 plugins, infinite-loop guest, host-call flood, deadline expiry | Reactor or Rayon starvation, reentry deadlock, late state commit |
| Storage and generation | Chunk loading, generation, lighting, saves during active load | Background pool starvation, stale snapshot publication, I/O backlog |

## Proposed acceptance measurements

These are targets to validate, not measured current results. The final release envelope may be narrower if the workload cannot satisfy them on the stated host.

- **Tick:** p99 authoritative tick duration at or below 50 ms for the declared 20 TPS workload, reported per active region and for world/global coordination. Show the distribution and maximum, not only an average.
- **Delivery:** publish p50/p95/p99 input-to-commit and commit-to-client-send latency for nearby movement, block placement, and required state transitions. Report visible-state age when replaceable movement is coalesced.
- **Correctness:** no lost or duplicated entity transfers; no stale command committed under a new ownership generation; cancellation and collision results match the declared compatibility contract; deterministic replay yields expected authoritative state.
- **Bounded resources:** ingress, plugin, background-job, per-client egress, and global egress byte limits remain effective. RSS and queue age do not grow without bound during a soak or slow-reader attack.
- **Isolation:** a saturated hotspot or timed-out WASM plugin does not delay unrelated regions beyond their published latency budget. Required transactions follow their documented timeout policy.
- **Protocol:** Java zlib and Bedrock transport interoperate with the targeted clients. Per-connection encryption and ordered required packets remain correct while frames are shared where compatible.

## Capacity accounting

For each profile, report the following together: physical cores and CPU utilization; memory and retained frame bytes; NIC line rate and payload/wire throughput; movers, observers, deliveries per second, and compressed frame cache hit rate; active entities and chunks; background job backlog; plugin decision and host-call rates. Record whether the server or the load generator was the limiting component.

The crowd calculation from the [overview](../architecture/overview.md) is a lower-bound planning tool: 500 movers visible to 499 others at 20 updates per second create 4.99 million deliveries/s. If the mandatory protocol and visibility contract requires more bytes or CPU than the host can supply, report that limit. Do not convert coalesced visual updates into a claim that every observer received every intermediate move.

## Publication format

A capacity claim should name the upstream-derived commit, complete workload manifest, benchmark duration, p99 tick and delivery results, correctness results, RSS, queue ceilings, CPU, and wire bandwidth. Describe any explicit gameplay or replication policy differences from the baseline. The resulting statement is an evidence-backed envelope for that configuration, not an unconditional 10,000-player guarantee.
