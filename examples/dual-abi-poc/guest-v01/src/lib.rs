wit_bindgen::generate!({ path: "../wit", world: "legacy", generate_all });
struct Plugin;
impl Guest for Plugin {
    fn run() -> f32 {
        demo::api0_1_0::player::heal(4.0)
    }
}
export!(Plugin);
