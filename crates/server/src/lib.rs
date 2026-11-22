//! Bounded lobby and authoritative fixed-rate simulation server over QUIC.
//!
//! [`Coordinator`] is independent of sockets and clocks. [`serve`] and [`serve_with_config`]
//! run it over a pure-Rust QUIC endpoint with TLS 1.3 encryption and compact binary framing.

mod coordinator;
mod layout;
mod runtime;

pub use coordinator::{
    ClientId, Config, Coordinator, Delivery, MAX_CATCH_UP_TURNS, MAX_FUTURE_TICKS,
};
pub use layout::{RANDOM_MAP_NAME, random_setup};
pub use runtime::{serve, serve_with_config};
