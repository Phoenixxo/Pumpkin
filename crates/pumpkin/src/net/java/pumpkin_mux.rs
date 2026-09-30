//! The `pumpkin:mux` endpoint for Pumpkin Patch clients.
//!
//! All Pumpkin Patch mod traffic travels over one custom payload channel. During configuration the
//! server sends `HELLO` with the mods it offers, the client answers with `REPLY`, and the server
//! assigns a route to each available mod in `ACCEPT`. In play, `DATA` frames carry opaque payloads
//! for one mod's channel.
//!
//! Server plugins reach a mod's traffic through the existing custom payload API. A `DATA` frame
//! arrives as a `PlayerCustomPayloadEvent` on the virtual channel `pumpkin:mux/<mod-id>/<channel>`,
//! and a plugin sends to a client component by calling `send_custom_payload` on that same virtual
//! channel. The virtual channel never reaches the wire.

use std::sync::Arc;

use bytes::Bytes;
use pumpkin_config::networking::PumpkinMuxConfig;
use pumpkin_protocol::java::client::play::CCustomPayload;
use tracing::{debug, info, warn};

use crate::{
    entity::player::Player, net::ClientPlatform,
    plugin::player::player_custom_payload::PlayerCustomPayloadEvent, server::Server,
};

/// The custom payload channel carrying every mux frame.
pub const CHANNEL: &str = "pumpkin:mux";
/// The prefix of the virtual channels server plugins use for mod traffic.
pub const VIRTUAL_PREFIX: &str = "pumpkin:mux/";
/// The wire format version this server speaks.
pub const MUX_VERSION: i32 = 1;

const MAX_MODS: usize = 256;
const MAX_CHANNELS: usize = 64;
const MAX_STRING: usize = 32_767;

/// A client's answer for one offered mod.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModStatus {
    Available,
    Missing,
    IncompatibleVersion,
    IncompatibleWorld,
    IncompatibleProtocol,
    RejectedLocally,
    DisabledByUser,
}

impl ModStatus {
    const fn from_byte(b: u8) -> Option<Self> {
        Some(match b {
            0 => Self::Available,
            1 => Self::Missing,
            2 => Self::IncompatibleVersion,
            3 => Self::IncompatibleWorld,
            4 => Self::IncompatibleProtocol,
            5 => Self::RejectedLocally,
            6 => Self::DisabledByUser,
            _ => return None,
        })
    }

    const fn to_byte(self) -> u8 {
        match self {
            Self::Available => 0,
            Self::Missing => 1,
            Self::IncompatibleVersion => 2,
            Self::IncompatibleWorld => 3,
            Self::IncompatibleProtocol => 4,
            Self::RejectedLocally => 5,
            Self::DisabledByUser => 6,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelloMod {
    pub id: String,
    pub version_req: String,
    pub world: String,
    pub required: bool,
    pub protocol: String,
    pub channels: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyMod {
    pub id: String,
    pub status: ModStatus,
    pub version: String,
    pub detail: String,
}

/// One frame of the `pumpkin:mux` wire format, version 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    Hello {
        mux_version: i32,
        server_id: String,
        mods: Vec<HelloMod>,
    },
    Reply {
        mux_version: i32,
        mods: Vec<ReplyMod>,
    },
    Accept {
        join: bool,
        message: String,
        routes: Vec<(String, i32)>,
    },
    Data {
        route: i32,
        channel: i32,
        payload: Vec<u8>,
    },
    Close {
        route: i32,
        reason: String,
    },
}

#[derive(Debug)]
pub struct MalformedFrame(pub &'static str);

fn put_var_int(out: &mut Vec<u8>, value: i32) {
    #[expect(clippy::cast_sign_loss)]
    let mut v = value as u32;
    loop {
        if v & !0x7F == 0 {
            #[expect(clippy::cast_possible_truncation)]
            out.push(v as u8);
            return;
        }
        #[expect(clippy::cast_possible_truncation)]
        out.push(((v & 0x7F) | 0x80) as u8);
        v >>= 7;
    }
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    #[expect(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    put_var_int(out, s.len() as i32);
    out.extend_from_slice(s.as_bytes());
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn var_int(&mut self) -> Result<i32, MalformedFrame> {
        let mut value: u32 = 0;
        for i in 0..5 {
            let (&b, rest) = self.0.split_first().ok_or(MalformedFrame("truncated varint"))?;
            self.0 = rest;
            value |= u32::from(b & 0x7F) << (7 * i);
            if b & 0x80 == 0 {
                #[expect(clippy::cast_possible_wrap)]
                return Ok(value as i32);
            }
        }
        Err(MalformedFrame("varint too long"))
    }

    fn count(&mut self, max: usize) -> Result<usize, MalformedFrame> {
        let n = usize::try_from(self.var_int()?).map_err(|_| MalformedFrame("negative count"))?;
        if n > max {
            return Err(MalformedFrame("count above limit"));
        }
        Ok(n)
    }

    fn byte(&mut self) -> Result<u8, MalformedFrame> {
        let (&b, rest) = self.0.split_first().ok_or(MalformedFrame("truncated byte"))?;
        self.0 = rest;
        Ok(b)
    }

    fn string(&mut self) -> Result<String, MalformedFrame> {
        let len = self.count(MAX_STRING)?;
        if self.0.len() < len {
            return Err(MalformedFrame("truncated string"));
        }
        let (s, rest) = self.0.split_at(len);
        self.0 = rest;
        String::from_utf8(s.to_vec()).map_err(|_| MalformedFrame("invalid utf-8"))
    }

    fn rest(&mut self) -> &'a [u8] {
        std::mem::take(&mut self.0)
    }

    fn end(&self) -> Result<(), MalformedFrame> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(MalformedFrame("trailing bytes"))
        }
    }
}

impl Frame {
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            Self::Hello {
                mux_version,
                server_id,
                mods,
            } => {
                put_var_int(&mut out, 0);
                put_var_int(&mut out, *mux_version);
                put_str(&mut out, server_id);
                #[expect(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                put_var_int(&mut out, mods.len() as i32);
                for m in mods {
                    put_str(&mut out, &m.id);
                    put_str(&mut out, &m.version_req);
                    put_str(&mut out, &m.world);
                    out.push(u8::from(m.required));
                    put_str(&mut out, &m.protocol);
                    #[expect(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                    put_var_int(&mut out, m.channels.len() as i32);
                    for c in &m.channels {
                        put_str(&mut out, c);
                    }
                }
            }
            Self::Reply { mux_version, mods } => {
                put_var_int(&mut out, 1);
                put_var_int(&mut out, *mux_version);
                #[expect(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                put_var_int(&mut out, mods.len() as i32);
                for m in mods {
                    put_str(&mut out, &m.id);
                    out.push(m.status.to_byte());
                    put_str(&mut out, &m.version);
                    put_str(&mut out, &m.detail);
                }
            }
            Self::Accept {
                join,
                message,
                routes,
            } => {
                put_var_int(&mut out, 2);
                out.push(u8::from(!*join));
                put_str(&mut out, message);
                #[expect(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                put_var_int(&mut out, routes.len() as i32);
                for (id, route) in routes {
                    put_str(&mut out, id);
                    put_var_int(&mut out, *route);
                }
            }
            Self::Data {
                route,
                channel,
                payload,
            } => {
                put_var_int(&mut out, 3);
                put_var_int(&mut out, *route);
                put_var_int(&mut out, *channel);
                out.extend_from_slice(payload);
            }
            Self::Close { route, reason } => {
                put_var_int(&mut out, 4);
                put_var_int(&mut out, *route);
                put_str(&mut out, reason);
            }
        }
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, MalformedFrame> {
        let mut r = Reader(bytes);
        let frame = match r.var_int()? {
            0 => {
                let mux_version = r.var_int()?;
                let server_id = r.string()?;
                let n = r.count(MAX_MODS)?;
                let mut mods = Vec::with_capacity(n);
                for _ in 0..n {
                    let id = r.string()?;
                    let version_req = r.string()?;
                    let world = r.string()?;
                    let required = r.byte()? != 0;
                    let protocol = r.string()?;
                    let c = r.count(MAX_CHANNELS)?;
                    let channels = (0..c).map(|_| r.string()).collect::<Result<_, _>>()?;
                    mods.push(HelloMod {
                        id,
                        version_req,
                        world,
                        required,
                        protocol,
                        channels,
                    });
                }
                Self::Hello {
                    mux_version,
                    server_id,
                    mods,
                }
            }
            1 => {
                let mux_version = r.var_int()?;
                let n = r.count(MAX_MODS)?;
                let mut mods = Vec::with_capacity(n);
                for _ in 0..n {
                    let id = r.string()?;
                    let status =
                        ModStatus::from_byte(r.byte()?).ok_or(MalformedFrame("unknown status"))?;
                    let version = r.string()?;
                    let detail = r.string()?;
                    mods.push(ReplyMod {
                        id,
                        status,
                        version,
                        detail,
                    });
                }
                Self::Reply { mux_version, mods }
            }
            2 => {
                let join = r.byte()? == 0;
                let message = r.string()?;
                let n = r.count(MAX_MODS)?;
                let mut routes = Vec::with_capacity(n);
                for _ in 0..n {
                    routes.push((r.string()?, r.var_int()?));
                }
                Self::Accept {
                    join,
                    message,
                    routes,
                }
            }
            3 => {
                let route = r.var_int()?;
                let channel = r.var_int()?;
                return Ok(Self::Data {
                    route,
                    channel,
                    payload: r.rest().to_vec(),
                });
            }
            4 => Self::Close {
                route: r.var_int()?,
                reason: r.string()?,
            },
            _ => return Err(MalformedFrame("unknown frame type")),
        };
        r.end()?;
        Ok(frame)
    }
}

/// One negotiated mod on a connection.
#[derive(Debug, Clone)]
pub struct MuxRoute {
    pub route: i32,
    pub mod_id: String,
    pub channels: Vec<String>,
    pub open: bool,
}

/// The routes negotiated for one connection. It lives as long as the connection.
#[derive(Debug, Clone, Default)]
pub struct MuxSession {
    pub routes: Vec<MuxRoute>,
}

impl MuxSession {
    fn by_route(&self, route: i32) -> Option<&MuxRoute> {
        self.routes.iter().find(|r| r.route == route && r.open)
    }

    fn by_mod(&self, mod_id: &str) -> Option<&MuxRoute> {
        self.routes.iter().find(|r| r.mod_id == mod_id && r.open)
    }
}

/// Where a connection is in the mux handshake.
#[derive(Debug, Default)]
pub enum Handshake {
    /// The mux is disabled, or the handshake has not started.
    #[default]
    Off,
    /// `HELLO` was sent and no `REPLY` has arrived.
    HelloSent,
    /// `ACCEPT` with `JOIN` was sent.
    Joined(MuxSession),
    /// The client never answered `HELLO`, so it is not a Pumpkin Patch client.
    NonPumpkin,
}

#[must_use]
pub fn hello(config: &PumpkinMuxConfig) -> Frame {
    Frame::Hello {
        mux_version: MUX_VERSION,
        server_id: format!("Pumpkin {}", env!("CARGO_PKG_VERSION")),
        mods: config
            .mods
            .iter()
            .map(|m| HelloMod {
                id: m.id.clone(),
                version_req: m.version_req.clone(),
                world: m.world.clone(),
                required: m.required,
                protocol: m.protocol.clone(),
                channels: m.channels.clone(),
            })
            .collect(),
    }
}

/// The server's decision after a `REPLY`.
pub struct Negotiated {
    pub accept: Frame,
    /// `None` when the join is refused.
    pub session: Option<MuxSession>,
}

/// Assigns routes to available mods, or refuses when a required mod is unavailable.
#[must_use]
pub fn negotiate(config: &PumpkinMuxConfig, reply: &[ReplyMod]) -> Negotiated {
    let mut routes = Vec::new();
    let mut missing = Vec::new();
    let mut next_route = 1;
    for offered in &config.mods {
        let answer = reply.iter().find(|r| r.id == offered.id);
        if answer.is_some_and(|a| a.status == ModStatus::Available) {
            routes.push(MuxRoute {
                route: next_route,
                mod_id: offered.id.clone(),
                channels: offered.channels.clone(),
                open: true,
            });
            next_route += 1;
        } else if offered.required {
            let reason = answer.map_or_else(
                || "not reported".to_string(),
                |a| format!("{:?}: {}", a.status, a.detail),
            );
            missing.push(format!("{} {} ({reason})", offered.id, offered.version_req));
        }
    }
    if !missing.is_empty() {
        return Negotiated {
            accept: Frame::Accept {
                join: false,
                message: format!("This server requires {}", missing.join(", ")),
                routes: Vec::new(),
            },
            session: None,
        };
    }
    Negotiated {
        accept: Frame::Accept {
            join: true,
            message: String::new(),
            routes: routes
                .iter()
                .map(|r| (r.mod_id.clone(), r.route))
                .collect(),
        },
        session: Some(MuxSession { routes }),
    }
}

#[must_use]
pub fn virtual_channel(mod_id: &str, channel: &str) -> String {
    format!("{VIRTUAL_PREFIX}{mod_id}/{channel}")
}

/// Splits `pumpkin:mux/<mod-id>/<channel>` into the mod id and the channel.
#[must_use]
pub fn parse_virtual_channel(channel: &str) -> Option<(&str, &str)> {
    channel.strip_prefix(VIRTUAL_PREFIX)?.rsplit_once('/')
}

/// Handles one serverbound `pumpkin:mux` frame during play.
pub fn handle_play_frame(server: &Arc<Server>, player: &Arc<Player>, data: &[u8]) {
    let ClientPlatform::Java(client) = player.client.as_ref() else {
        return;
    };
    let frame = match Frame::decode(data) {
        Ok(frame) => frame,
        Err(MalformedFrame(why)) => {
            debug!("Dropping malformed pumpkin:mux frame from {}: {why}", player.gameprofile.name);
            return;
        }
    };
    match frame {
        Frame::Data {
            route,
            channel,
            payload,
        } => {
            let target = {
                let session = client.mux.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                session.as_ref().and_then(|s| {
                    let r = s.by_route(route)?;
                    let ch = r.channels.get(usize::try_from(channel).ok()?)?;
                    Some(virtual_channel(&r.mod_id, ch))
                })
            };
            let Some(target) = target else {
                debug!("Dropping pumpkin:mux DATA for unknown route {route} channel {channel}");
                return;
            };
            let mut event =
                PlayerCustomPayloadEvent::new(player.clone(), target, Bytes::from(payload));
            server.plugin_manager.fire_blocking(server, &mut event);
        }
        Frame::Close { route, reason } => {
            let mut session = client.mux.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(r) = session
                .as_mut()
                .and_then(|s| s.routes.iter_mut().find(|r| r.route == route))
            {
                r.open = false;
                info!(
                    "Pumpkin Patch closed route {route} ({}) for {}: {reason}",
                    r.mod_id, player.gameprofile.name
                );
            }
        }
        other => warn!("Unexpected pumpkin:mux frame in play: {other:?}"),
    }
}

/// Sends a plugin's virtual-channel payload as a `DATA` frame.
///
/// Returns `false` when `channel` is not a mux virtual channel, so the caller sends it as an
/// ordinary custom payload.
pub async fn intercept_send(player: &Player, channel: &str, data: &[u8]) -> bool {
    let Some((mod_id, name)) = parse_virtual_channel(channel) else {
        return false;
    };
    let ClientPlatform::Java(client) = player.client.as_ref() else {
        return true;
    };
    let frame = {
        let session = client.mux.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        session.as_ref().and_then(|s| {
            let r = s.by_mod(mod_id)?;
            let index = r.channels.iter().position(|c| c == name)?;
            Some(Frame::Data {
                route: r.route,
                channel: i32::try_from(index).ok()?,
                payload: data.to_vec(),
            })
        })
    };
    match frame {
        Some(frame) => {
            let bytes = frame.encode();
            player
                .send_client_packet(&CCustomPayload::new(CHANNEL, &bytes))
                .await;
        }
        None => debug!(
            "Dropping {channel} for {}: no open pumpkin:mux route",
            player.gameprofile.name
        ),
    }
    true
}

#[cfg(test)]
mod tests {
    use pumpkin_config::networking::PumpkinMuxModConfig;

    use super::*;

    fn config() -> PumpkinMuxConfig {
        PumpkinMuxConfig {
            enabled: true,
            mods: vec![
                PumpkinMuxModConfig {
                    id: "example:ping".into(),
                    version_req: "^0.1".into(),
                    required: true,
                    protocol: "ping/1".into(),
                    channels: vec!["request".into(), "response".into()],
                    ..Default::default()
                },
                PumpkinMuxModConfig {
                    id: "example:extra".into(),
                    protocol: "extra/1".into(),
                    ..Default::default()
                },
            ],
        }
    }

    #[test]
    fn frames_round_trip() {
        let frames = [
            hello(&config()),
            Frame::Reply {
                mux_version: 1,
                mods: vec![ReplyMod {
                    id: "example:ping".into(),
                    status: ModStatus::Available,
                    version: "0.1.0".into(),
                    detail: String::new(),
                }],
            },
            Frame::Accept {
                join: true,
                message: String::new(),
                routes: vec![("example:ping".into(), 1)],
            },
            Frame::Data {
                route: 300,
                channel: 1,
                payload: vec![0, 1, 2, 255],
            },
            Frame::Close {
                route: 1,
                reason: "trap".into(),
            },
        ];
        for frame in frames {
            assert_eq!(Frame::decode(&frame.encode()).unwrap(), frame);
        }
    }

    #[test]
    fn malformed_frames_are_rejected() {
        assert!(Frame::decode(&[]).is_err());
        assert!(Frame::decode(&[9]).is_err());
        assert!(Frame::decode(&[4, 1, 5, b'a']).is_err());
        assert!(Frame::decode(&[2, 0, 0, 0, 7]).is_err());
    }

    #[test]
    fn negotiation_assigns_routes_to_available_mods() {
        let reply = [
            ReplyMod {
                id: "example:ping".into(),
                status: ModStatus::Available,
                version: "0.1.0".into(),
                detail: String::new(),
            },
            ReplyMod {
                id: "example:extra".into(),
                status: ModStatus::Missing,
                version: String::new(),
                detail: String::new(),
            },
        ];
        let n = negotiate(&config(), &reply);
        assert_eq!(
            n.accept,
            Frame::Accept {
                join: true,
                message: String::new(),
                routes: vec![("example:ping".into(), 1)]
            }
        );
        assert_eq!(n.session.unwrap().routes.len(), 1);
    }

    #[test]
    fn negotiation_refuses_missing_required_mod() {
        let n = negotiate(&config(), &[]);
        assert!(matches!(n.accept, Frame::Accept { join: false, .. }));
        assert!(n.session.is_none());
    }

    #[test]
    fn virtual_channels_split_on_last_slash() {
        let v = virtual_channel("example:ping", "response");
        assert_eq!(v, "pumpkin:mux/example:ping/response");
        assert_eq!(parse_virtual_channel(&v), Some(("example:ping", "response")));
        assert_eq!(parse_virtual_channel("minecraft:brand"), None);
    }
}
