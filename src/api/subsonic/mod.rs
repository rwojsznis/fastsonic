//! The Subsonic / OpenSubsonic client.
//!
//! Everything here talks to the music server the user runs.
//!
//! | | |
//! |---|---|
//! | `types` | the server's response shapes, vendored |
//! | `auth` | the salted-token credential, and the redaction rule |
//! | `client` | the transport: one envelope, one error mapping |
//! | `calls` | one method per endpoint, then the calls the app makes |
//! | `convert` | adapters into the app's own `api::models` vocabulary (D5) |
//! | `scrobble` | when to tell the server what is playing |
//! | `native` | the few Navidrome calls Subsonic cannot answer (D7) |
//! | `live` | tests that need `migration/devserver` |
//!
//! While the migration is in flight this lives beside the Spotify client it
//! replaces; `migration/PROGRESS.md` says which phase that is.

pub mod auth;
pub mod calls;
pub mod client;
pub mod convert;
#[cfg(test)]
mod live;
pub mod native;
pub mod scrobble;
pub mod types;

pub use auth::{Credentials, redacted};
pub use client::{ApiError, SubsonicClient};
pub use native::{NativeClient, NativeError, NativeSession, SignIn};
pub use scrobble::{Report, Scrobbler};
pub use types::*;
