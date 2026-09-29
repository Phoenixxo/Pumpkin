# Phase 03 - Region ownership

This phase develops the write-ownership model in [spatial ownership and ECS](../architecture/spatial-ownership-and-ecs.md). It uses the workload from [phase 00](00-baseline-and-benchmarks.md), the observer and egress interfaces from [phase 01](01-spatial-index-and-networking.md), and the plugin invocation contract from [phase 02](02-plugin-execution.md). [Phase 04](04-ecs-migration.md) then places entity columns under the same ownership rules.

## Outcome and current baseline

Each migrated cell and entity needs one exclusive mutation owner. Independent regions can then advance without a world-wide tick join. A managed, stackless task drives each plugin-capable region operation; Tokio handles network and I/O, and bounded Rayon work covers only audited, plugin-free CPU leaves. Adjacent regions exchange committed read-only snapshots and owner-routed commands rather than share mutable world state. The Global scheduler topology remains a working fallback while Region ownership is developed.

Upstream at `4426d1113` has a dedicated [joined server ticker](../../crates/pumpkin/src/server/ticker.rs), Rayon parallelism across [worlds](../../crates/pumpkin/src/server/mod.rs) and within [world ticks](../../crates/pumpkin/src/world/mod.rs), and shared `World` fields. It has no region actor. The upstream [`pumpkin-scheduler` crate](../../crates/pumpkin-scheduler/src/domain.rs) names execution domains but documents only Global admission and is not wired into `pumpkin`. Region ownership therefore requires a new integration, even though the scheduler provides vocabulary and infrastructure to extend.

## Ownership contract

The contract separates exclusive mutation from routing and read-only observation. It also makes stale messages and slow consumers visible, which is necessary before any cell can move between owners.

| Contract | Rule |
|---|---|
| Mutation authority | Only the current owner for `(world, cell, generation)` may change chunk, block, scheduled-tick, or entity state. Mutable APIs require an owner token or route a command to that owner. |
| Message identity | Every command carries an origin, sequence, target tick, relevant entity key, and observed owner generation. A command with a stale generation is forwarded once or rejected explicitly. |
| Tick scheduling | A region has one authoritative writer and at most one commit in flight. Its managed task can suspend at an awaitable boundary after releasing mutable state borrows. Region deadlines remain independent, while global tasks publish timestamped commands or snapshots. |
| Border reads | Halo snapshots name their source generation and committed tick. Each subsystem declares acceptable age, and strict writes go to the owning region. |
| Plugin decisions | A pending decision suspends its managed task without holding a Rayon worker, Store borrow, or world lock. Same-region host requests follow a documented staged-read and ordered-write rule. The v0.1 compatibility lane retains its graph-wide admission rule. |
| Capacity | Mailboxes have item and byte budgets. Background jobs have admission limits separate from simulation work. |

Ownership applies to a group of cells that can interact within the required consistency boundary; a chunk-sized mutex would not express that boundary. A dense cluster may need one owner while distant regions advance independently. [Folia's regionizer](https://docs.papermc.io/folia/reference/region-logic/) also merges nearby regions and splits independent ones. Pumpkin's design should therefore be evaluated through measured ownership cost, job scheduling, plugin behavior, and replication rather than an inaccurate claim that Folia uses static regions.

## Implementation sequence

The migration starts by identifying existing writers. It then introduces routing and one-owner execution before moving border reads, background jobs, and eventually whole cells or entities between owners.

### 3.1 Inventory mutation paths and define commands

Trace writes from packets, scheduled block ticks, entity AI, collisions, redstone, lighting, generation, commands, plugin host calls, persistence, and world lifecycle into shared `World` state. Classify each operation as owner-local, cross-owner strict, snapshot-computed, or world/global. Typed `WorldCommand` variants and `OwnerToken`-gated mutation methods can then make authority explicit without changing external behavior. Unmigrated operations may still use the existing world path, but **each object must have exactly one authoritative path** throughout the transition.

Synchronous neighbor-chunk loads need particular attention because they can expand a region into another owner's cells during its tick. Each such operation must use a declared halo, request asynchronous loading, or enter a documented multi-cell operation before it mutates state.

### 3.2 Add the directory and bounded mailboxes

A read-mostly `(world, cell) -> (region, generation)` directory can publish `ArcSwap` snapshots or an equivalent atomic view. It routes commands but does not own mutable world state. Each region receives a bounded mailbox with distinct policies for required commands and replaceable observations. A generational entity locator serves packets and plugin calls. Stale-route retries, mailbox occupancy and bytes, and oldest command age show when the routing layer falls behind.

### 3.3 Schedule exclusive region ticks

Extend upstream's [`SchedulerService`](../../crates/pumpkin-scheduler/src/scheduler.rs) with Region admission and bounded task accounting only after the Global mode and v0.2 plugin boundary are proven. Keep state ownership separate from scheduler poll serialization: the [domain contract](../../crates/pumpkin-scheduler/src/domain.rs) allows tasks to interleave when they suspend, so admission alone does not make a multi-step mutation atomic. Route a plugin-capable input into a managed region task before any awaitable plugin call. The owner reads a bounded input batch, completes a short synchronous state step, and releases its mutable borrows before it awaits a plugin result or an owner-routed host operation. It checks the owner generation and transaction revision before applying the result. Other commands can queue while the transaction is pending; an explicit ordering rule decides whether independent work can advance.

Within a synchronous step, the owner may send immutable snapshots or disjoint batches to Rayon for sufficiently large, audited plugin-free calculations. The leaf returns data, never a live owner borrow, plugin invocation, or host-call future. The managed task incorporates that data under the owner token. Rayon workers must not call plugins or wait on plugin and host requests through `block_on`; an accidental plugin-capable leaf is rejected and resubmitted through the managed scheduler. This avoids trapping a worker behind an async dependency and keeps v0.1 admission valid across execution domains.

Global time, weather, scoreboards, and other world-level services retain explicit owners and send timestamped updates to regions. The replacement ticker also needs defined behavior for `/tick freeze`, sprint, shutdown, save, and world unload.

### 3.4 Publish border snapshots and isolate background jobs

Begin with a small dirty-section halo containing only the block, light, collision, and entity data needed by the migrated subsystem. The owner publishes it after commit and labels it with source generation and revision. A background result is accepted only while its inputs satisfy that subsystem's validity rule. Lighting, AI pathfinding, and chunk generation each need bounded CPU admission when their job is demonstrably plugin-free; a path that can enter plugin code starts in a managed task instead. The existing [bounded chunk generation scheduler](../../crates/pumpkin-world/src/chunk_system/schedule.rs) remains in place, and background work must leave capacity for simulation.

### 3.5 Implement entity and cell handoff

A handoff begins at a committed tick boundary. The old owner finishes its in-flight tick and buffers new commands while chunks, entities, scheduled work, pending plugin decisions, and mailbox cursors move to the new owner. Only after acknowledgement does the directory publish the new generation. Jobs based on old revisions are discarded or recomputed. Commands sent through the old route retain their original sequence and have a forwarding hop limit, preventing loss or duplicate processing of a moving player.

Split and merge decisions should use hysteresis over p99 tick cost, mailbox age, and cross-boundary interaction rate. A dense combat or redstone island stays together when splitting would create more transactions than it removes. Adjacent player-less chunks can remain independently owned despite a nearby hotspot.

### 3.6 Roll out by subsystem and world area

Start with one constrained subsystem in a controlled world area. Compare its output with the current path in shadow or replay mode, then make the owner authoritative behind a feature flag. Block and entity transitions follow once their behavior is equivalent. A rollback drains owner state into the compatibility representation at a tick boundary, so the two representations never become simultaneous authoritative writers.

## Validation gate

The gate checks both semantic equivalence and isolation. A faster region tick cannot count as progress if migration loses work or a busy region stalls unrelated owners.

| Evidence | Required result |
|---|---|
| Ownership assertions | No active cell or entity may have two writers, and no command may mutate state under a stale generation. |
| Deterministic replay | Given the same ordered inputs, strict block and entity operations must have equivalent outcomes across worker counts and scheduling delays. Any deliberate ordering change needs a documented and tested contract. |
| Migration stress | Repeated split, merge, teleport, chunk unload/reload, shutdown, and save cycles must neither duplicate nor lose entities, scheduled work, or required commands. Inject worker stalls and transfer failures. |
| Cross-border behavior | Collision, redstone, lighting, AI, block placement, and plugin host calls must obey their declared snapshot or strict-operation semantics at boundaries. |
| Plugin boundary | A region root that can call a plugin begins as a managed task; v0.1 admission covers its full causal chain. Suspend and resume without holding a world, resource-table, or Store borrow, and reject stale plugin results after handoff. The same traces pass in Global fallback mode. |
| Hotspot isolation | Under the [phase 00](00-baseline-and-benchmarks.md) 500-player reference event, unrelated regions must meet established p99 tick and message-latency budgets. Report the busy region's mandatory serial time and replication load separately. |
| Resource bounds | Mailbox bytes, background queue age, halo bytes, and Rayon occupancy must remain within configured caps during long soaks. |

Completion requires both correctness and measured isolation. A 20 TPS target gives a 50 ms tick interval, but a fixed host may still exceed it under an arbitrary all-to-all hotspot. [Phase 05](05-capacity-validation.md) states the hardware and workload envelope that actually passes.
