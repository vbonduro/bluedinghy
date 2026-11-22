//! The crate's error type.

use std::error::Error as StdError;

/// Prefix of the D-Bus error names BlueFerry documents for its own failures.
const BLUEFERRY_ERROR_PREFIX: &str = "io.weirdware.BlueFerry.Error.";

type Source = Box<dyn StdError + Send + Sync + 'static>;

/// Everything that can go wrong talking to BlueFerry.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// BlueFerry isn't on the bus: the service isn't running, or the session
    /// bus has no owner for its name.
    #[error("BlueFerry is not running on the D-Bus session bus")]
    NotRunning,

    /// BlueFerry understood the call and refused it.
    ///
    /// `code` is the documented error name without its
    /// `io.weirdware.BlueFerry.Error.` prefix, e.g. `NotReady` (phone not
    /// connected), `RateLimited`, `InvalidArgs`, `SendFailed` or
    /// `SendOutcomeUnknown`. `SendOutcomeUnknown` means the message may still have
    /// been sent, so don't blindly retry it.
    #[error("BlueFerry refused the request ({code}): {message}")]
    Refused { code: String, message: String },

    /// The D-Bus layer failed (connection lost, timeout, ...).
    #[error("D-Bus error: {0}")]
    Dbus(#[source] Source),

    /// BlueFerry answered, but not in a shape this crate understands.
    #[error("unexpected response from BlueFerry: {0}")]
    Protocol(#[source] Source),

    /// A string couldn't be used as an address.
    #[error(transparent)]
    Address(#[from] crate::AddressError),
}

impl Error {
    /// The BlueFerry error code (see [`Error::Refused`]), if this is one.
    pub fn code(&self) -> Option<&str> {
        match self {
            Self::Refused { code, .. } => Some(code),
            _ => None,
        }
    }
}

impl From<zbus::Error> for Error {
    fn from(error: zbus::Error) -> Self {
        use zbus::fdo;
        match &error {
            zbus::Error::MethodError(name, message, _) => {
                let name = name.as_str();
                if let Some(code) = name.strip_prefix(BLUEFERRY_ERROR_PREFIX) {
                    return Self::Refused {
                        code: code.to_owned(),
                        message: message.clone().unwrap_or_default(),
                    };
                }
                if matches!(
                    name,
                    "org.freedesktop.DBus.Error.ServiceUnknown"
                        | "org.freedesktop.DBus.Error.NameHasNoOwner"
                ) {
                    return Self::NotRunning;
                }
            }
            zbus::Error::FDO(inner) => {
                if matches!(
                    **inner,
                    fdo::Error::ServiceUnknown(_) | fdo::Error::NameHasNoOwner(_)
                ) {
                    return Self::NotRunning;
                }
            }
            _ => {}
        }
        Self::Dbus(Box::new(error))
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::Protocol(Box::new(error))
    }
}

/// Shorthand for `Result<T, bluedinghy::Error>`.
pub type Result<T, E = Error> = std::result::Result<T, E>;
