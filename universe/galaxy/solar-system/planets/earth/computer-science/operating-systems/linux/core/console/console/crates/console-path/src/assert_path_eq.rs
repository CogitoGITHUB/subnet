//! Path equality in Rust is defined by comparing their `components()`. However,
//! `Path::components()` will perform its own normalization, which makes
//! `assert_eq!` not suitable testing.
//!
//! This module provides two macros, `assert_path_eq!` and `assert_path_ne!`,
//! which converts path to string before comparison. They accept PathBuf, Path,
//! String, and &str as parameters.

#[macro_export]
macro_rules! assert_path_eq {
    ($left:expr, $right:expr $(,)?) => {
        assert_eq!(
            AsRef::<Path>::as_ref(&$left).to_str().unwrap(),
            AsRef::<Path>::as_ref(&$right).to_str().unwrap()
        )
    };
}

#[macro_export]
macro_rules! assert_path_ne {
    ($left:expr, $right:expr $(,)?) => {
        assert_ne!(
            AsRef::<Path>::as_ref(&$left).to_str().unwrap(),
            AsRef::<Path>::as_ref(&$right).to_str().unwrap()
        )
    };
}

