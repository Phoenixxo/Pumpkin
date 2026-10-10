wit_bindgen::generate!({ path: "../wit", world: "modern", generate_all });
struct Plugin;
impl Guest for Plugin {
    async fn run() -> f32 {
        demo::api0_2_0::player::heal(6.0).await
    }
}
export!(Plugin);
