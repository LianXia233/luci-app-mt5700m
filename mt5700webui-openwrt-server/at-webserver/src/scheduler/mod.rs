//! Scheduling layer.
//!
//! `arbiter` is the single AT request queue (priority + retry + cancellation),
//! `jobs` owns task/periodic lifecycles, `gate` is the read cache gate and
//! `plan` carries the day/night band-lock plan.

pub mod arbiter;
pub mod gate;
pub mod jobs;
pub mod plan;
