use serde::{Deserialize, Serialize};

/// Configuration for the `pumpkin:mux` channel used by Pumpkin Patch clients.
///
/// When enabled, the server offers the listed client mods during the configuration phase and
/// routes mod traffic between Pumpkin Patch client components and server plugins.
#[derive(Deserialize, Serialize, Default, Clone)]
#[serde(default)]
pub struct PumpkinMuxConfig {
    /// Whether the server sends the `pumpkin:mux` handshake to Java clients.
    pub enabled: bool,
    /// The client mods this server offers, in the order they appear in the handshake.
    pub mods: Vec<PumpkinMuxModConfig>,
}

/// One client mod the server offers in the `pumpkin:mux` handshake.
#[derive(Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct PumpkinMuxModConfig {
    /// The mod id, such as `example:ping`.
    pub id: String,
    /// The version requirement, such as `^0.1`.
    pub version_req: String,
    /// The client WIT world the mod targets.
    pub world: String,
    /// Whether a client without this mod is refused.
    pub required: bool,
    /// The mod-owned protocol token, which must equal the client manifest's token.
    pub protocol: String,
    /// The mod's channels. A channel's index in this list is its index on the wire.
    pub channels: Vec<String>,
}

impl Default for PumpkinMuxModConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            version_req: "*".to_string(),
            world: "pumpkin:client/client-mod@0.1.0".to_string(),
            required: false,
            protocol: String::new(),
            channels: Vec::new(),
        }
    }
}
