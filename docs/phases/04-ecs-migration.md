# Phase 04 — Owner-local ECS migration

**Status:** implementation plan, not current behavior. [Phase 03](03-region-ownership.md) establishes exclusive region ownership; [spatial ownership and ECS](../architecture/spatial-ownership-and-ecs.md) defines the proposed data layout. [Phase 00](00-baseline-and-benchmarks.md) supplies the comparison workload, and [phase 05](05-capacity-validation.md) evaluates the full capacity target.

## Outcome and baseline

Move the hot transform, velocity, hitbox, and spatial-membership paths to cache-friendly columns while preserving UUIDs, protocol entity IDs, plugin handles, physics, collision, and lifecycle behavior. Upstream at `4426d1113` uses `ArcSwap<Vec<Arc<dyn EntityBase>>>` and atomics/locks in [world](../../crates/pumpkin/src/world/mod.rs) and [entity](../../crates/pumpkin/src/entity/mod.rs). World ticks already use Rayon, so the goal is to make that work cheaper and safely owner-local, not to add a parallel iterator around the existing pointer graph.

## Migration sequence

### 4.1 Establish stable identity and a compatibility facade

Introduce a generational internal `EntityKey` and locator mapping to `(world, region, batch, row, owner_generation)`. Keep external UUID and protocol entity IDs stable across compaction and region transfer. Wrap current `EntityBase` lookups behind a facade that resolves the key through the owner; plugin and packet APIs do not receive a raw row pointer that can outlive a structural mutation. Count stale lookups and generation failures.

### 4.2 Pilot hot SoA columns

Implement aligned batches containing sequential position, velocity, and hitbox arrays. Keep sparse or cold data—inventory, AI memory, scoreboard links, plugin-specific metadata—in separate storage. Preserve position precision and collision rules until compatibility traces prove a deliberate change safe. Batch sizes, alignment, and storage library choice are selected by profile; `hecs`, `bevy_ecs`, or a small purpose-built SoA each have different scheduling and structural-change costs.

### 4.3 Make systems explicit

Define read and write sets for movement, broad-phase collision, narrow-phase collision, combat, AI, and tracking. A Rayon `par_iter_mut` over disjoint batches can update independent columns or produce `MotionIntent` values from immutable collision halos. Interacting entities are resolved by their owner in a stable order. Do not have two workers mutate the same entity or both sides of a collision directly. If an ECS library already schedules systems from component access, avoid an additional unconstrained Rayon scheduler over the same systems.

### 4.4 Batch structural changes and transfers

Collect spawn, despawn, mount, component-add/remove, and cross-region movement requests during parallel work. At the owner commit point, apply them in deterministic order and update the locator atomically with the batch row changes. Ensure `ArcSwap` compatibility snapshots, if retained temporarily, cannot become a second authoritative store. Transfer whole component rows and pending commands at the [phase 03](03-region-ownership.md) handoff boundary; old owner generations reject stale jobs.

### 4.5 Migrate one entity family at a time

Start with a family whose movement and collision behavior is covered by deterministic replay. Run old and new systems in shadow mode against the same immutable inputs without double-applying output. Compare state hashes, collision candidates, packet deltas, and plugin-visible fields. Expand to other entities, players, vehicles, and complex mounting behavior after parity is shown. Keep rollback at a tick boundary while a family is still dual-represented.

## Memory and concurrency invariants

1. A component row belongs to one region owner at one generation; no `&mut` access escapes the owner or overlaps a Rayon batch.
2. External entity identity survives row compaction and transfer. A reused slot has a new generation, invalidating stale messages.
3. Structural changes occur only after all parallel query borrows finish. Cross-entity results are sorted before owner commit.
4. Read-only snapshots publish committed data only. A long-running AI or lighting result must match its source revision and owner generation before it can affect ECS state.
5. Memory includes component capacity, sparse stores, compatibility facades, and retained snapshots; lower per-entity CPU is not enough if RSS becomes unstable.

## Validation gate

| Evidence | Required result |
| --- | --- |
| Gameplay parity | Movement, collision, damage, mounting, spawn/despawn, and plugin-visible state match the declared baseline trace for each migrated family. |
| Ownership safety | Tests and debug assertions detect no dual writer, stale-row mutation, or accepted old-generation command under transfer and compaction stress. |
| Determinism | The same ordered inputs yield the same authoritative state under one and many Rayon workers, excluding explicitly documented nondeterministic observations. |
| Performance | Profile cycles/entity tick, LLC misses, allocation rate, lock/atomic contention, rows moved, and p99 region tick against [phase 00](00-baseline-and-benchmarks.md). Dense and sparse worlds both appear in the comparison. |
| Lifecycle | Repeated spawn/despawn, teleport, dimension change, vehicle/passenger transitions, save/load, and region migration do not lose or duplicate entities. |

The phase is complete when correctness passes and the fixed workload shows a measured improvement without increasing p99 tick time or unbounded memory. It does not itself prove 10,000-player capacity; that belongs to [phase 05](05-capacity-validation.md).
