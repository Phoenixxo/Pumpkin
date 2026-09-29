# Phase 04 - Owner-local ECS migration

[Phase 03](03-region-ownership.md) establishes exclusive region ownership. This phase uses it to migrate hot entity data to the layout described in [spatial ownership and ECS](../architecture/spatial-ownership-and-ecs.md). The comparison workload comes from [phase 00](00-baseline-and-benchmarks.md); [phase 05](05-capacity-validation.md) evaluates the full capacity target.

## Outcome and baseline

The proposed layout places transforms, velocities, hitboxes, and spatial membership in cache-friendly columns. UUIDs, protocol entity IDs, plugin handles, physics, collision, and lifecycle behavior remain compatible. Upstream at `4426d1113` stores entities through `ArcSwap<Vec<Arc<dyn EntityBase>>>` with atomics and locks in [world](../../crates/pumpkin/src/world/mod.rs) and [entity](../../crates/pumpkin/src/entity/mod.rs). World ticks already use Rayon. The opportunity is to reduce pointer chasing and contention within owner-local work, rather than add another parallel iterator around the existing pointer graph.

## Migration sequence

Entity identity must become independent of storage location before columns can move or compact. The sequence then pilots a small set of hot fields, makes system access explicit, and migrates entity families only after replay shows equivalent behavior.

### 4.1 Establish stable identity and a compatibility facade

An internal generational `EntityKey` should resolve through a locator to `(world, region, batch, row, owner_generation)`. External UUIDs and protocol entity IDs stay stable when rows compact or cross regions. A compatibility facade can resolve existing `EntityBase` lookups through the owner. Plugin host calls use opaque handles with owner and generation checks; neither plugin nor packet APIs retain row pointers, table borrows, or mutable component references across structural changes or an await. Counters for stale lookups and generation failures reveal incorrect lifetimes.

### 4.2 Pilot hot SoA columns

The pilot stores positions, velocities, and hitboxes in sequential aligned arrays. Inventory, AI memory, scoreboard links, and plugin-specific metadata remain in separate sparse or cold storage. Position precision and collision rules stay intact unless compatibility traces justify a deliberate change. Profiles should determine batch size, alignment, and whether `hecs`, `bevy_ecs`, or a small purpose-built structure-of-arrays fits best; each has different scheduling and structural-change costs.

### 4.3 Make systems explicit

Movement, broad-phase collision, narrow-phase collision, combat, AI, and tracking each need declared read and write sets and an explicit plugin-entry classification. Rayon `par_iter_mut` over disjoint batches can update independent columns or derive `MotionIntent` values from immutable collision halos only for audited plugin-free systems. A system that can dispatch an event, invoke a plugin, or await a host operation runs from a managed owner task and releases ECS borrows before suspension. The owner resolves interactions between entities in a stable order, so two workers never mutate the same entity or opposite sides of a collision concurrently. If the selected ECS library schedules systems from component access, its scheduler should coordinate with Rayon rather than run a second unconstrained schedule over the same data.

### 4.4 Batch structural changes and transfers

Parallel systems collect spawn, despawn, mount, component-add/remove, and cross-region movement requests instead of changing table shape mid-query. The owner applies those requests in deterministic order at commit and updates the locator atomically with row changes. If `ArcSwap` compatibility snapshots remain temporarily, they serve readers but cannot become a second authoritative store. At the [phase 03](03-region-ownership.md) handoff boundary, whole component rows and pending commands transfer together; old owner generations reject stale jobs.

### 4.5 Migrate one entity family at a time

The first family should have movement and collision behavior covered by deterministic replay. Run old and new systems in shadow mode over the same immutable inputs, applying only one output. Compare state hashes, collision candidates, packet deltas, and plugin-visible fields before extending the migration to other entities, players, vehicles, or complex mounts. While a family is dual-represented, rollback remains a tick-boundary operation.

## Memory and concurrency invariants

1. A component row belongs to one region owner at one generation; no `&mut` access escapes the owner or overlaps a Rayon batch.
2. External entity identity survives row compaction and transfer. A reused slot has a new generation, invalidating stale messages.
3. Structural changes occur only after all parallel query borrows finish. Cross-entity results are sorted before owner commit.
4. Read-only snapshots publish committed data only. A long-running AI or lighting result must match its source revision and owner generation before it can affect ECS state.
5. Memory includes component capacity, sparse stores, compatibility facades, and retained snapshots; lower per-entity CPU is not enough if RSS becomes unstable.
6. A plugin-capable ECS system begins in a managed domain task. No Rayon leaf enters a plugin, and no row, owner-state, Store, or resource-table borrow survives an await.

## Validation gate

The gate requires gameplay parity and stable ownership before treating lower cycles per entity as a useful improvement.

| Evidence | Required result |
| --- | --- |
| Gameplay parity | Movement, collision, damage, mounting, spawn/despawn, and plugin-visible state must match the declared baseline trace for each migrated family. |
| Ownership safety | Tests and debug assertions must detect any dual writer, stale-row mutation, or accepted old-generation command under transfer and compaction stress. |
| Plugin integration | Mixed v0.1/v0.2 callbacks and host operations resolve opaque entity handles after suspension, preserve legacy causal admission, and reject stale generations without retaining an ECS borrow. |
| Determinism | The same ordered inputs must yield the same authoritative state with one or many Rayon workers, apart from explicitly documented nondeterministic observations. |
| Performance | Profile cycles per entity tick, LLC misses, allocation rate, lock and atomic contention, rows moved, and p99 region tick against [phase 00](00-baseline-and-benchmarks.md). Compare both dense and sparse worlds. |
| Lifecycle | Repeated spawn/despawn, teleport, dimension change, vehicle/passenger transitions, save/load, and region migration must not lose or duplicate entities. |

Completion requires correctness and a measured improvement on the fixed workload without higher p99 tick time or unbounded memory growth. [Phase 05](05-capacity-validation.md) tests whether those improvements translate into 10,000-player capacity.
