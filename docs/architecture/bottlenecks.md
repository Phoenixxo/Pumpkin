# Bottlenecks, mitigations, and proof points

[Documentation index](../README.md) · [Overview](overview.md) · [Spatial ownership and ECS](spatial-ownership-and-ecs.md) · [Networking](networking.md) · [Plugin pipeline](plugin-pipeline.md)

The proposed design trades some shared-state contention for actor mailboxes, ownership transfers, snapshots, and output queues. Each optimization must be judged by end-to-end p99 latency and correctness, not only a faster isolated microbenchmark. [Phase 00](../phases/00-baseline-and-benchmarks.md) establishes the baseline; [phase 05](../phases/05-capacity-validation.md) is the capacity gate.

| Pressure point | Cause or risk | Mitigation | Evidence to collect |
| --- | --- | --- | --- |
| Joined tick | Slowest world or phase delays the current ticker | Independent region schedules and timestamped global updates | p50/p95/p99 tick duration, lateness by region, unaffected-region progress during a hotspot |
| Single hot owner | Dense collision, combat, redstone, or plugin decisions are truly serial | Keep strict interaction island minimal; parallelize AI plans, entity batches, encoding, and lighting outside it | Mandatory serial CPU time, cross-owner message count, region backlog age |
| Dynamic split/merge | Frequent repartitioning copies state and invalidates routing | Cost-based hysteresis and tick-boundary generation handoff | Migrations per minute, transfer bytes, stale command rejections, replay equivalence |
| Cross-owner commands | Fine grids increase messages and can introduce cycles | Merge strongly coupled cells; owned messages with causal IDs, deadlines, and deterministic commit ordering | Messages per interaction, wait graph, deadline misses, consistency assertions |
| Global or chunk locks | A lock held across player scans, generation, or awaits serializes unrelated work | Short control-plane locks; actor-owned mutation; immutable published halos | Lock wait time, hold time, blocked cores |
| Snapshot publication | Whole-world copies would consume bandwidth and memory | Dirty-section or chunk snapshots and small border halos; reuse immutable `Arc` values | Snapshot bytes per tick, retained generations, stale-result rate |
| Cache misses and false sharing | Trait objects, per-field atomics, and interleaved entity state | Owner-local SoA batches; separate hot/cold components; align and size batches by profile | LLC misses, cycles/entity tick, cache-line contention |
| ECS structural churn | Spawning, despawning, or adding components moves rows | Defer structural changes to owner commit and batch by archetype | Rows moved per tick, allocation count, spawn/despawn p99 |
| Tiny Rayon jobs | Queue and stealing overhead exceeds useful work | Minimum batch sizes and a finite simulation pool; background admission budget | Scheduling overhead/job, worker utilization, useful CPU fraction |
| Generation and lighting | Chunk generation or inline light propagation consumes active tick time | Retain bounded generation pool; versioned lighting jobs; reserve simulation CPU | Background queue age, active-region p99, lighting correctness at publication |
| Observer discovery | World-wide scans scale with all players, not local viewers | Section-to-observer index and exact visibility filter | Candidates versus actual recipients, lookup CPU |
| All-to-all broadcast | Dense crowds create quadratic deliveries | Per-client and global byte budgets; coalesce only replaceable updates; tune interest and cadence policy | Deliveries/s, bytes/s, visible-state age, required-packet latency |
| Repeated compression | Identical Java payloads are zlib-framed for each connection | Cache frames by protocol, compression profile, and content revision | Compression cycles, cache hit rate, frame count |
| Per-client cipher | Java CFB8 and transport encryption are connection stateful | One writer per connection; batch writes; reserve crypto CPU | Cycles/byte, socket backpressure, writer queue age |
| Queue retention | Per-client caps do not bound aggregate memory | Weighted global and per-client byte permits; slow-client policy | Retained frame bytes, RSS, queue age, eviction count |
| WASM guest CPU | Async host imports do not preempt a guest tight loop | Fuel/epoch yielding or trap, wall deadline, memory and host-call quotas | Guest CPU/invocation, traps, deadline misses, reactor latency |
| Plugin decision ordering | Cancellable hooks must return before commit | Park transaction only; release ownership borrow; validate version on reply | Origin wait time, late replies, cancellation parity |
| Transitional global plugin gate | One fair gate preserves compatibility but limits parallel throughput | Use as a v0.1 safety bridge; move eligible v0.2 work to domain-aware admission | Gate occupancy, queue age, event throughput |

The upstream [ticker](../../crates/pumpkin/src/server/ticker.rs), [world state](../../crates/pumpkin/src/world/mod.rs), [Java outgoing writer](../../crates/pumpkin/src/net/java/outgoing.rs), [plugin dispatch](../../crates/pumpkin/src/plugin/mod.rs), and [Global scheduler scaffold](../../crates/pumpkin-scheduler/src/global.rs) are measurement anchors. These table entries are design risks and proposed controls; no row asserts a measured speedup.
