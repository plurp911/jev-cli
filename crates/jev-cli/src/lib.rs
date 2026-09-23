//! Internals of the `jev` binary.
//!
//! # This is not a library
//!
//! This crate is `publish = false` and promises **no** API. The library target exists
//! for three internal reasons and no others:
//!
//! 1. the integration tests can drive the command surface in process, without spawning;
//! 2. the fuzz targets in `fuzz/` can reach the parsers, which is where hostile input
//!    lands;
//! 3. the benchmark harness can measure parsing and rendering separately from process
//!    startup.
//!
//! The supported interface of this project is the `jev` command line. See
//! `docs/adr/0003-cli-compatibility.md`. Anything here may change in any commit.

pub mod batch;
pub mod cli;
pub mod commands;
pub mod context;
pub mod dataset;
pub mod digest;
pub mod errors;
pub mod exit;
pub mod gate;
pub mod input;
pub mod interrupt;
pub mod mcp;
pub mod metrics;
pub mod ordered;
pub mod output;
pub mod paths;
pub mod render;
pub mod request;
