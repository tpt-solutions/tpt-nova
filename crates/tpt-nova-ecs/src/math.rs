//! Re-exports of the math types used throughout the ECS.
//!
//! We depend on [`glam`] for SIMD-friendly linear algebra rather than hand
//! rolling matrices and quaternions.

pub use glam::{Mat4, Quat, Vec3};
