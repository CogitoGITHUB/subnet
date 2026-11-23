//! Re-exports signature-aware named flag helpers from `console-protocol`.
//!
//! Kept as a thin module so existing `crate::named_flags::…` call sites stay stable.

pub use console_protocol::engine::named_flags::*;
