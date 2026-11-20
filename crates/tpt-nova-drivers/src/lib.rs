//! Hot-swappable behavior drivers for TPT Nova (Phase 8, Layers 1–3).
//!
//! A [`Driver`] implements behavior for one component type. Drivers are
//! registered in a [`DriverRegistry`] under `(component, version)`, and one
//! version per component is active. Swapping the active version is atomic:
//! the next frame's [`DriverRegistry::snapshot`] sees the new driver, and
//! frames already in flight keep the one they started with.
//!
//! - **Layer 1:** the trait, identity/versioning, capability set, and registry.
//! - **Layer 2 (partial):** activation is gated by a granted capability policy
//!   ([`DriverRegistry::with_policy`]). Native `.so` loading and a WASM sandbox
//!   are not implemented.
//! - **Layer 3 (partial):** [`DriverRegistry::rollback`] reverts to the previously
//!   active version, and [`migrate::migrate`] composes schema upgrades that
//!   require a registered migration for every hop. Shadow execution is not
//!   implemented.
//!
//! Layer 4 (AI self-debugging and symbolic verification) is not implemented.

pub mod driver;
pub mod migrate;
pub mod registry;
#[cfg(feature = "wasm-sandbox")]
pub mod sandbox;

pub use driver::{
    Capability, CapabilitySet, Driver, DriverContext, DriverError, DriverId, DriverVersion,
};
pub use migrate::{migrate, MigrationError};
pub use registry::{DriverHandle, DriverRegistry, RegistryError, RegistryState};
