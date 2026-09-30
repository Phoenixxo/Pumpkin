use pumpkin_data::packet::clientbound::config::DISCONNECT;
use pumpkin_macros::java_packet;

use crate::ClientPacket;
use pumpkin_util::text::TextComponent;
use crate::ser::NetworkWriteExt;
use pumpkin_util::version::JavaMinecraftVersion;

/// Disconnects the client during configuration. Since 1.20.3 the reason is a text component,
/// encoded the same way as the play-state disconnect.
#[java_packet(DISCONNECT)]
pub struct CConfigDisconnect<'a> {
    pub reason: &'a TextComponent,
}

impl<'a> CConfigDisconnect<'a> {
    #[must_use]
    pub const fn new(reason: &'a TextComponent) -> Self {
        Self { reason }
    }
}

impl ClientPacket for CConfigDisconnect<'_> {
    fn write_packet_data(
        &self,
        mut write: impl std::io::Write,
        version: &JavaMinecraftVersion,
    ) -> Result<(), crate::ser::WritingError> {
        write.write_component(self.reason, version)
    }
}
