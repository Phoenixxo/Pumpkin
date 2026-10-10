use std::sync::{Arc, Mutex};
use wasmtime::component::{Access, Accessor, Component, HasSelf, Linker};
use wasmtime::{Config, Engine, Store};

mod v01 {
    wasmtime::component::bindgen!({
        path: "../wit", world: "legacy",
        imports: { default: async | store | trappable },
        exports: { default: async },
    });
}
mod v02 {
    wasmtime::component::bindgen!({
        path: "../wit", world: "modern",
        imports: { default: async | store | trappable },
    });
}

// One operation implementation, shared by both host paths.
struct Operations {
    health: Mutex<f32>,
}
impl Operations {
    async fn heal(&self, amount: f32) -> wasmtime::Result<f32> {
        tokio::task::yield_now().await;
        let mut health = self
            .health
            .lock()
            .map_err(|_| wasmtime::format_err!("health lock poisoned"))?;
        *health = (*health + amount).min(20.0);
        Ok(*health)
    }
}
struct State {
    operations: Arc<Operations>,
}
impl v01::demo::api0_1_0::player::Host for State {}
impl v02::demo::api0_2_0::player::Host for State {}

// Sync guest ABI, async Rust host bridge. The guest calls without `.await`.
impl v01::demo::api0_1_0::player::HostWithStore<State> for HasSelf<State> {
    async fn heal(mut access: Access<'_, State, Self>, amount: f32) -> wasmtime::Result<f32> {
        let operations = access.get().operations.clone();
        operations.heal(amount).await
    }
}
// Async guest ABI: scoped access before suspension.
impl v02::demo::api0_2_0::player::HostWithStore<State> for HasSelf<State> {
    async fn heal(accessor: &Accessor<State, Self>, amount: f32) -> wasmtime::Result<f32> {
        let operations = accessor.with(|mut access| access.get().operations.clone());
        operations.heal(amount).await
    }
}

async fn run_plugin(
    engine: &Engine,
    linker: &Linker<State>,
    path: &str,
    operations: Arc<Operations>,
) -> wasmtime::Result<f32> {
    let component = Component::from_file(engine, path)?;
    let imports: Vec<_> = component
        .component_type()
        .imports(engine)
        .map(|(name, _)| name.to_owned())
        .collect();
    let old = imports.iter().any(|name| name == "demo:api/player@0.1.0");
    let new = imports.iter().any(|name| name == "demo:api/player@0.2.0");
    let mut store = Store::new(engine, State { operations });
    match (old, new) {
        (true, false) => {
            let plugin = v01::Legacy::instantiate_async(&mut store, &component, linker).await?;
            plugin.call_run(&mut store).await
        }
        (false, true) => {
            let plugin = v02::Modern::instantiate_async(&mut store, &component, linker).await?;
            store
                .run_concurrent(async |accessor| plugin.call_run(accessor).await)
                .await?
        }
        _ => wasmtime::bail!("unsupported or mixed demo API versions"),
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> wasmtime::Result<()> {
    let mut config = Config::new();
    config.wasm_component_model_async(true);
    let engine = Engine::new(&config)?;
    let mut linker = Linker::new(&engine);
    v01::Legacy::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)?;
    v02::Modern::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)?;
    let operations = Arc::new(Operations {
        health: Mutex::new(10.0),
    });
    let old = run_plugin(
        &engine,
        &linker,
        "components/guest-v01.wasm",
        operations.clone(),
    )
    .await?;
    let new = run_plugin(&engine, &linker, "components/guest-v02.wasm", operations).await?;
    assert_eq!((old, new), (14.0, 20.0));
    println!("v0.1 sync guest: health={old}");
    println!("v0.2 async guest: health={new}");
    Ok(())
}
