//! Rust API adapter to a running [BlueFerry] instance.
//!
//! BlueDinghy hooks into BlueFerry's D-Bus interface and provides a Rust
//! API to send and receive iMessages over a connected iPhone.
//!
//! ```no_run
//! # async fn demo() -> Result<(), bluedinghy::Error> {
//! use bluedinghy::{Address, BlueDinghy};
//! use futures_util::StreamExt;
//!
//! let dinghy = BlueDinghy::new().await?;
//!
//! // Subscribe first, then read history, so nothing falls in a gap.
//! let mut incoming = dinghy.watch().await?;
//! let recent = dinghy.recent_messages(10).await?;
//!
//! dinghy.send(&Address::parse("+14165550123")?, "hello").await?;
//!
//! while let Some(message) = incoming.next().await {
//!     println!("{}", message?.text);
//! }
//! # Ok(()) }
//! ```
//!
//! [BlueFerry]: https://github.com/erikwb/blueferry

mod address;
mod bluedinghy;
mod dbus;
mod error;
mod history;
mod types;
mod watch;

pub use address::{Address, AddressError, AddressKind};
pub use bluedinghy::BlueDinghy;
pub use error::{Error, Result};
pub use types::{Conversation, ConversationId, Message, MessageId, TransferId};
pub use watch::{MessageStream, WatchOptions};
