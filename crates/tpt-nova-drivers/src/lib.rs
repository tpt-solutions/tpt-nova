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
//!   ([`DriverRegistry::with_policy`]). WASM drivers run in a sandbox (`wasm-sandbox`).
//!   Native `.so`/`.dll` drivers load with `native-loader`, but they are trusted
//!   and not sandboxed.
//! - **Layer 4 (internal):** [`monitor`] and [`anomaly`] observe invariant
//!   violations and drift, [`verify`] proves one-tick affine models with an exact
//!   internal Fourier-Motzkin solver over rationals, [`compile`] turns a
//!   declarative fix into a driver and its model from one spec, and [`repair`]
//!   runs verify -> shadow -> preserve -> deploy -> learn. [`compile::GainSearch`]
//!   supplies patches. No external solver or AI service is used.
//! - **Layer 3:** [`DriverRegistry::rollback`] reverts to the previously
//!   active version, [`migrate::migrate`] composes schema upgrades that
//!   require a registered migration for every hop, and [`shadow`] runs a
//!   candidate against a cloned world beside the baseline and promotes it only
//!   when the sampled entities agree.
//!

pub mod anomaly;
pub mod compile;
pub mod driver;
pub mod migrate;
pub mod monitor;
#[cfg(feature = "native-loader")]
pub mod native;
pub mod registry;
pub mod repair;
#[cfg(feature = "wasm-sandbox")]
pub mod sandbox;
pub mod shadow;
pub mod verify;

pub use anomaly::{Anomaly, AnomalyTracker};
pub use compile::{compile as compile_fix, FixSpec, FloatField, GainSearch};
pub use driver::{
    Capability, CapabilitySet, Driver, DriverContext, DriverError, DriverId, DriverVersion,
};
pub use migrate::{migrate, MigrationError};
pub use monitor::{Invariant, Monitor, ValueBound, Violation};
#[cfg(feature = "native-loader")]
pub use native::{NativeDriver, NativeLoadError};
pub use registry::{DriverHandle, DriverRegistry, RegistryError, RegistryState};
pub use repair::{
    repair, Candidate, ClosureDriver, FixMemory, Proposer, RejectReason, RejectedFix, RepairError,
    RepairOutcome, RepairSite,
};
pub use shadow::{promote_if_clean, shadow_compare, ShadowError, ShadowReport};
pub use verify::{verify_affine, AffineModel, Bounds, Verified, VerifyError};
