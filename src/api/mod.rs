//! The music server's API, and the vocabulary the rest of the app speaks.
//!
//! [`subsonic`] is the client: one transport, one credential, one server.
//! [`models`] is what it adapts into — `Track`, `Album`, `Playlist`,
//! `Page<T>` — which is the vocabulary every page of the interface is
//! written in and which deliberately did not change when the server behind
//! it did (D5 in `migration/00-decisions.md`).
//!
//! There is no gateway and no second identity. The old service had two application
//! identities with different quotas and per-playlist write access; a
//! self-hosted server has one set of credentials and no quota, so D6 dropped
//! the routing layer entirely.

pub mod activity;
pub mod models;
pub mod subsonic;

pub use activity::NetActivity;
pub use models::PlayRequest;
pub use subsonic::{ApiError, Credentials, SubsonicClient};
