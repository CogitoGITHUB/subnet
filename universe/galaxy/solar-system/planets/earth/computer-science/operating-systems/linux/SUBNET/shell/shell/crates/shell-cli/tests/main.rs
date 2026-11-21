#![allow(clippy::unwrap_used)]

mod commands;
mod completions;
mod highlight_use;
mod last_result;

#[macro_use]
extern crate shell_test_support;
use shell_test_support::harness::main;
