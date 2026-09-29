# Phase 3 — Region ownership

**Status:** implementation plan, not completed work. [Phase 0](00-baseline-and-benchmarks.md) supplies the workload and performance envelope; [phase 1](01-spatial-index-and-networking.md) supplies spatial lookups and bounded egress; [phase 2](02-plugin-execution.md) supplies an execution-domain contract for plugin calls. This phase implements the write-ownership model described in [spatial ownership and ECS](../architecture/spatial-ownership-and-ecs.md). [Phase 4](04-ecs-migration.md) uses that model for entity columns.

## Outcome and current baseline

At the end of this phase, migrated cells and entities have one exclusive mutation owner, and independent regions can advance without a world-wide tick join. Tokio schedules and routes work; bounded Rayon tasks run synchronous simulation with exclusive state ownership. Adjacent read-only snapshots and owner-routed commands replace shared mutable access across region boundaries.

Upstream at `4426d1113` has a dedicated [joined server ticker](../../crates/pumpkin/src/server/ticker.rs), Rayon parallelism across [worlds](../../crates/pumpkin/src/server/mod.rs) and within [world ticks](../../crates/pumpkin/src/world/mod.rs), and shared `World` fields. No region actor is present. The upstream [`pumpkin-scheduler` crate](../../crates/pumpkin-scheduler/src/domain.rs) defines execution-domain names but documents Global admission only and is not wired into `pumpkin`; it is infrastructure to extend, not an already working region scheduler.

## Required contracts before migration

| Contract | Rule |
|---|---|
| Mutation authority | Only the current owner for `(world, cell, generation)` can change chunk, block, scheduled-tick, or entity state. Mutable APIs require an owner token or route a command. |
| Message identity | Every command carries an origin, sequence, target tick, entity key when relevant, and observed owner generation. A stale generation is forwarded once or rejected explicitly. |
| Tick scheduling | At most one CPU tick task per region. Region tick deadlines are independent; global tasks publish timestamped commands or snapshots. |
| Border reads | Halo snapshots name source generation and committed tick. Each subsystem declares its permitted age; strict writes go to the owning region. |
| Plugin decisions | A pending decision parks a transaction, not a Rayon worker or world lock. Same-region host requests have a documented staged-read/ordered-write rule. |
| Capacity | Mailboxes have item and byte budgets; background jobs have separate admission limits from simulation. |

These rules are intentionally stronger than “put a mutex around each chunk.” A cluster of mutually interacting cells may need one owner, while distant regions advance independently. [Folia's regionizer](https://docs.papermc.io/folia/reference/region-logic/) already merges nearby regions and splits independent ones; the Pumpkin design must be judged on measured ownership, job, plugin, and replication behavior rather than a claim that Folia is static.

## Work packages

### 3.1 Inventory mutation paths and define commands

Trace calls from packets, scheduled block ticks, entity AI, collisions, redstone, lighting, generation, commands, plugin host calls, persistence, and world lifecycle into shared `World` state. Classify each operation as owner-local, cross-owner strict, snapshot-computed, or world/global domain. Introduce typed `WorldCommand` variants and `OwnerToken`-gated mutation methods without changing external behavior. A compatibility route can send unmigrated operations to the existing world path while their state is not yet owner-managed; **each object must have exactly one authoritative path**.

Audit calls that synchronously load neighbor chunks. A region must not expand into another owner's cells during its tick. Such a call either uses a declared halo, requests asynchronous loading, or enters a documented multi-cell operation before mutation.

### 3.2 Add the directory and bounded mailboxes

Publish a read-mostly `(world, cell) -> (region, generation)` directory using `ArcSwap` or an equivalent atomic snapshot. The directory is a routing index, not mutable world state. Give each region a bounded mailbox with separate required-command and replaceable-observation policies. Provide a generational entity locator for packets and plugin calls. Add counters for stale-route retries, mailbox occupancy, bytes, and oldest command age.

### 3.3 Schedule exclusive region ticks

Extend upstream's [`SchedulerService`](../../crates/pumpkin-scheduler/src/scheduler.rs) with Region admission and bounded task accounting, while keeping the region state owner separate from scheduler poll serialization. The [domain contract](../../crates/pumpkin-scheduler/src/domain.rs) permits task interleaving on suspension, so it does not make a multi-step world mutation atomic by itself. A Tokio coordinator collects a bounded input batch and transfers the region's boxed state into a Rayon task. The task performs a synchronous simulation step, including any nested, sufficiently large Rayon batches, and returns `(state, output)` by a oneshot. The coordinator queues messages while the state is in flight and cannot submit another tick for that region. It applies output to plugin and replication dispatchers after regaining the state. No Rayon worker calls `block_on` to wait for an event or host request.

Keep global time, weather, scoreboards, and other world-level services under explicit owners. Their updates arrive at regions as timestamped messages. Define `/tick freeze`, sprint, shutdown, save, and world unload behavior before replacing the existing ticker path.

### 3.4 Publish border snapshots and isolate background jobs

Start with a small dirty-section halo containing only the block, light, collision, and entity data needed by the migrated subsystem. Publish after owner commit. Label snapshots with source generation and revision; accept a job result only when its inputs still satisfy that subsystem's validity rule. Lighting, AI pathfinding, and chunk generation use their own bounded CPU admissions. Preserve the current [bounded chunk generation scheduler](../../crates/pumpkin-world/src/chunk_system/schedule.rs); do not allow background work to occupy all simulation capacity.

### 3.5 Implement entity and cell handoff

Cut over at a committed tick boundary. Finish the old owner's in-flight tick, buffer newly arriving commands, transfer chunks, entities, scheduled work, pending plugin decisions, and mailbox cursors, then wait for new-owner acknowledgement before publishing a new directory generation. Late jobs with old revisions are discarded or recomputed. Old-route commands are forwarded with a hop limit and their original sequence, so a moving player is neither lost nor processed twice.

Use split/merge hysteresis based on p99 tick cost, mailbox age, and cross-boundary interaction rate. A dense combat or redstone island stays together if splitting would create more transactions than it removes. Do not lock adjacent, player-less chunks simply because a hotspot is active nearby.

### 3.6 Roll out by subsystem and world area

Enable ownership first for one constrained subsystem and a controlled world area. Compare its output with the current path in shadow/replay mode, then make the owner authoritative behind a feature flag. Expand to block and entity transitions only after equivalent behavior is demonstrated. Maintain a safe rollback path that drains owner state back into the compatibility representation at a tick boundary; never run two authoritative writers for the same object.

## Validation gate

| Evidence | Required result |
|---|---|
| Ownership assertions | No active cell or entity has two writers; no command mutates state under a stale generation. |
| Deterministic replay | Given the same ordered inputs, strict block/entity operations have equivalent outcomes across worker counts and scheduling delays. Where ordering is intentionally changed, document and test the new contract. |
| Migration stress | Repeated split, merge, teleport, chunk unload/reload, shutdown, and save cycles neither duplicate nor lose entities, scheduled work, or required commands. Inject worker stalls and transfer failures. |
| Cross-border behavior | Collision, redstone, lighting, AI, block placement, and plugin host calls obey their declared snapshot or strict-operation semantics at boundaries. |
| Hotspot isolation | Under the [phase 0](00-baseline-and-benchmarks.md) 500-player reference event, unrelated regions meet the established p99 tick and message-latency budgets. The busy region's mandatory serial time and replication load are reported separately. |
| Resource bounds | Mailbox bytes, background queue age, halo bytes, and Rayon occupancy remain within configured caps during long soaks. |

The phase is complete only when the correctness gate and the measured isolation gate pass. The 20 TPS target corresponds to a 50 ms tick interval, but it is not a guarantee for an arbitrary all-to-all hotspot on fixed hardware. The exact successful hardware and workload envelope belongs in [phase 5](05-capacity-validation.md).
