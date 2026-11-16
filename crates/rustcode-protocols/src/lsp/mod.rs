//! Language Server Protocol (LSP) types, framed stdio transport, client lifecycle, and manager.

pub mod client;
pub mod codec;
pub mod manager;
pub mod types;

pub use client::{LspClient, LspError};
pub use codec::{decode_framed_message, encode_framed_message, read_framed_message, write_framed_message, LspCodecError};
pub use manager::{LspClientManager, LspServerConfig};
pub use types::*;
