//! vpak-core: the deterministic substrate behind the `vpak` CLI.
//!
//! * [`manifest`]  — the `vpak.toml` manifest.
//! * [`archive`]   — pack a source tree into a `.vpak`, unpack and verify one.
//! * [`secrets`]   — the pack-time secret scan.
//! * [`install`]   — the install working directory: state, phases, journal,
//!   target, survey, constraints, plan, questions, audit.
//! * [`bootstrap`] — compile the system prompt handed to a runner.
//! * [`policy`]    — runner policy and budget.
//! * [`runner`]    — the `Runner` trait, the headless Claude runner, the shell runner.
//! * [`fleet`]     — hand an install off to a vflt collective.
//! * [`templates`] — embedded prompt templates.

pub mod archive;
pub mod bootstrap;
pub mod fleet;
pub mod install;
pub mod manifest;
pub mod policy;
pub mod runner;
pub mod secrets;
pub mod templates;
pub mod util;

pub use install::Workdir;
pub use manifest::Manifest;
pub use policy::{Budget, Policy};
pub use runner::{RunOutcome, RunRequest, Runner};
