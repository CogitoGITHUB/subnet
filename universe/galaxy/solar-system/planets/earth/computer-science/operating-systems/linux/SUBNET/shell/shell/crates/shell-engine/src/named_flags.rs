//! Re-exports signature-aware named flag helpers from `shell-protocol`.
//!
//! Kept as a thin module so existing `crate::named_flags::…` call sites stay stable.

pub use shell_protocol::engine::named_flags::*;
