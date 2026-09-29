# Phase 02 - Plugin runtime and managed execution

This phase applies the [plugin pipeline](../architecture/plugin-pipeline.md) to the workload defined in [phase 00](00-baseline-and-benchmarks.md). It follows the network bounds in [phase 01](01-spatial-index-and-networking.md) and establishes the plugin execution contract before [phase 03](03-region-ownership.md) adds region and entity routing.

## Outcome and source boundary

Pumpkin composes two sibling, generic crates for this work. `pumpkin-plugin-runtime` owns access to each WASM Store and its reentry policy; `pumpkin-scheduler` admits and drives managed game work. The application connects them through its plugin manager and host bindings. This phase keeps v0.1 plugins on a permanent, graph-wide `LegacySyncReentry` compatibility path and gives v0.2 components a physically async WIT and `ConcurrentAsync` Store-owner path. Plugin-capable roots start as managed, stackless Global-domain tasks. Region and Entity domains are reserved for the later ownership phase.

## Baseline and proposed ABI

The description of current upstream behavior remains pinned to `4426d1113`. At that revision, the [plugin manager](../../crates/pumpkin/src/plugin/mod.rs) awaits both mutable and immutable handler groups, and `fire_blocking` bridges synchronous callers into that path. The [WASM loader](../../crates/pumpkin/src/plugin/loader/wasm/mod.rs) shares a `LegacySyncReentry` policy among its Stores, while the [Store executor](../../crates/pumpkin-plugin-runtime/src/executor.rs) supplies bounded channels and causal reentry. The policy does not yet cover every native and WASM entrypoint through the plugin manager. Upstream's [`pumpkin-scheduler` crate](../../crates/pumpkin-scheduler/src/domain.rs) implements Global admission as a scaffold, but the server does not yet use it for plugin dispatch. [Movement](../../crates/pumpkin/src/net/java/play/player_position.rs) and [block placement](../../crates/pumpkin/src/block/registry.rs) still need plugin decisions before gameplay commits.

The separate [v0.2 WIT proposal on the fork](https://github.com/Phoenixxo/Pumpkin/tree/95fecc1b5969aa59dbb141f4c1090635f65ccd7e/crates/pumpkin-plugin-wit/v0.2) specifies the `pumpkin:plugin@0.2.0` package. It is absent from the pinned upstream checkout, and the WIT declaration alone does not provide a v0.2 host, runtime integration, or region scheduler. This phase implements that proposed ABI boundary.

## Implementation sequence

The work begins by locating every entry into the plugin graph. It then composes scheduler admission with the two Store policies, builds the v0.2 host, and applies resource and lifecycle limits to both paths. Plugin-free simulation and finite CPU work continue to use Rayon.

### 2.1 Inventory the complete callback graph

Trace native and WASM events, commands and suggestions, lifecycle callbacks, scheduled tasks, IPC, AI goals, chunk generation, and nested host calls. Record which callers require a returned value before commit, which calls can suspend, and where the current `fire_blocking` bridge occupies a worker. Include registration priority, returned-event mutation, cancellation, and caller ordering. This inventory must cover the full v0.2 callback surface, not only frequent packet events.

The fork's v0.2 WIT declares 14 async plugin callbacks and 955 async interface functions. The metadata export and two pure UUID conversions remain synchronous. Event registration keeps the `blocking` flag: `handle-event` takes and returns an owned event value, and the host applies the returned value for a blocking handler after the callback completes. A nonblocking handler's returned event is ignored. That flag does not itself promise detached delivery, post-commit publication, coalescing, or a deadline. Any such scheduling policy belongs to the host and needs its own compatibility and ordering rule.

### 2.2 Compose managed Global roots with Store ownership

Integrate the [Global scheduler](../../crates/pumpkin-scheduler/src/global.rs) so every plugin-capable root with registered callbacks enters a bounded managed task before it awaits a Store, host import, I/O operation, or another plugin. Preserve the no-handler fast path: when no callback is registered for an event, gameplay continues without creating a scheduler task or acquiring plugin admission. A suspended task releases its active poll; it does not retain a Rayon simulation worker or a world lock. `pumpkin-scheduler` manages domain admission and wakeup, while `pumpkin-plugin-runtime` remains responsible for exclusive Store access. Serial task polls do not make a multi-step world mutation atomic, so event state must be owned across suspension and revalidated before commit.

At this phase, Global is the implemented domain for plugin-capable roots. Host dispatch uses owned requests and replies and routes game-state operations through the current authoritative world path. It does not expose a Region or Entity identifier through WIT or pretend those owners already exist. [Phase 03](03-region-ownership.md) adds owner directories, generations, and narrower routing once the world can enforce them.

### 2.3 Preserve v0.1 as a permanent compatibility lane

Extend `LegacySyncReentry` admission to the entire reachable v0.1 callback graph, including native handlers and all manager entrypoints. Independent roots queue fairly; a nested `A → host → B → host → A` chain keeps its admission identity rather than reacquiring the gate. This serialization is the v0.1 behavior boundary, not a temporary step to remove when v0.2 becomes available. Mixed installations can run both ABIs, with each invocation following its own policy and with cross-ABI calls retaining their causal context.

The managed caller awaits a v0.1 lane result, but a synchronous compatibility worker may remain occupied until that legacy call returns. Keep `fire_blocking` only for callers that have not yet crossed an async boundary, account for those workers explicitly, and use bounded compatibility capacity separate from Rayon simulation workers. Migrate tick-critical callers before claiming that plugins cannot tie up simulation workers. Neither a second Tokio runtime nor recursive `block_on` solves reentry. A Rayon `yield_local` can reduce waiting pressure but is not the admission rule. Measure graph-gate occupancy, queue age, compatibility-worker occupancy, and the effect on small and crowded servers.

### 2.4 Implement the proposed v0.2 ABI and async host

Select v0.1 or v0.2 from the component's versioned WIT imports and exports. Reject unknown or mixed package versions; `metadata.version` describes the plugin release, not its ABI. Preserve the [v0.1 synchronous world](../../crates/pumpkin-plugin-wit/v0.1/plugin.wit) while generating bindings for the separate `pumpkin:plugin@0.2.0` package. Update host bindings, the WASM loader and host implementation, the SDK, and code-generated WIT data together.

Run v0.2 callbacks through a `ConcurrentAsync` Store-owner policy. The owner alone accesses its Store, while independently admitted roots can make progress when an async guest or host operation suspends. Native async bindings and the host dispatcher must use actual async operations for I/O, game-state routing, and cross-plugin calls; marking WIT imports async without changing blocking host implementations would leave the same stall. Opaque resource handles determine host-side ownership. Physical scheduler domains and IDs stay out of WIT, and no borrowed mutable event state or world guard crosses an awaited import. The v0.2 callback set includes lifecycle, events, commands and suggestions, tasks, IPC, AI goals, and chunk generation, so integration and latency budgets must cover all of them.

### 2.5 Bound execution and settle lifecycle outcomes

Configure fuel or epoch interruption in the pinned Wasmtime revision and prove with a tight-loop guest that the selected mechanism takes effect. Add wall deadlines, Store memory and table limits, bounded per-plugin invocation and host-call queues, and per-import limits for call count, decoded bytes, results, and time. Guest instruction fuel does not meter native host work. Run guest and compatibility execution on bounded capacity that cannot monopolize Tokio I/O workers or Rayon simulation workers.

A decision deadline fences authoritative commit and produces the event's documented failure result; a late reply must be ignored. Dropping the waiting future or reaching its deadline does **not** prove the guest stopped running. Hard cancellation requires a verified Store interruption or teardown path, and shutdown and unload must account for in-flight host work before releasing resources. Repeated violations should quarantine the plugin with diagnostics. Native plugins are outside the WASM sandbox and need cooperative behavior or process isolation for an equivalent hard limit.

## Validation gate

The phase gate tests the two ABIs separately and together. It must distinguish a timely host decision from guest termination and measure the cost of plugin machinery when little or no plugin work is present.

| Test | Required result |
| --- | --- |
| v0.1 parity | Native and WASM handler priority, returned-event changes, cancellation, client corrections, and synchronous compatibility behavior must match the declared legacy contract. Graph-wide admission handles independent and nested roots without deadlock. |
| v0.2 ABI | Versioned imports and exports select the matching package. Unknown or mixed versions are rejected, and plugin metadata cannot select an ABI. Blocking event results apply after completion; nonblocking returned values are ignored. |
| Callback coverage | Exercise all 14 async v0.2 callbacks, including commands, tasks, IPC, AI goals, and generation, together with representative async host imports, opaque resource handles, and the synchronous metadata export. A suspended import does not occupy a reactor or simulation worker. |
| Causal reentry and mixed ABIs | Opposing roots, nested `A → host → B → host → A`, and v0.1-to-v0.2 and v0.2-to-v0.1 calls finish or fail by policy while preserving causal identity and Store ownership. |
| Faults and lifecycle | Infinite loops, memory growth, host-call floods, unload, and shutdown stay within configured limits. A deadline rejects a late world commit; a separate interruption or teardown test proves when the guest has actually stopped. |
| Performance | Record p99 callback and decision wait, Global admission and v0.1 gate occupancy, Store and host queue age, compatibility-worker occupancy, and guest CPU or fuel. Compare plugin-free, small-server, and crowded profiles against phase 00. Prove that an event with no registered handler incurs no plugin task admission. A stalled guest must not consume unrelated Tokio I/O or Rayon simulation capacity beyond the declared budget. |

After these gates pass, [phase 03](03-region-ownership.md) can route host requests to World, Region, and Entity owners. Its owner generations and cross-border transaction rules build on the invocation and Store boundaries established here.
