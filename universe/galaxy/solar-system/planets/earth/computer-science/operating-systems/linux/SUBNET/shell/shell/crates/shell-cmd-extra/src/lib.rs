#![doc = include_str!("../README.md")]
pub mod extra;
pub use extra::*;

#[cfg(test)]
#[macro_use]
extern crate shell_test_support;

#[cfg(test)]
use shell_test_support::harness::main;
