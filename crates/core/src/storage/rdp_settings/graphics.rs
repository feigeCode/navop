use serde::{Deserialize, Serialize};

/// How the RDP client advertises the Graphics Pipeline Extension (EGFX) in its Client Core Data.
///
/// Some servers refuse to serve a session at all unless the capability is advertised (GNOME
/// Remote Desktop), while others accept it and may then encode the session with codecs this client
/// cannot decode (Windows). [`Auto`](Self::Auto) therefore connects without advertising it and
/// retries once when a server refuses such a connection, which keeps both kinds of server working.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RdpEgfxMode {
    /// Do not advertise the capability up front; retry with it when a server refuses the connection.
    #[default]
    Auto,
    /// Advertise the capability from the first attempt onwards.
    Always,
    /// Never advertise the capability and never retry.
    Never,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RdpGraphicsSettings {
    pub egfx: RdpEgfxMode,
}
