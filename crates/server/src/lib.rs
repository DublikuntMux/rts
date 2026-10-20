//! Bounded lobby and deterministic lockstep relay.
//!
//! [`Coordinator`] is independent of sockets and clocks. Native builds also expose
//! [`serve`] and [`serve_with_config`]. The runtime accepts plain `WebSockets`; terminate
//! TLS at a reverse proxy for browser WSS clients.

mod coordinator;

pub use coordinator::{ClientId, Config, Coordinator, Delivery, MAX_FUTURE_TICKS};

#[cfg(not(target_arch = "wasm32"))]
mod runtime;
#[cfg(not(target_arch = "wasm32"))]
pub use runtime::{serve, serve_with_config};
