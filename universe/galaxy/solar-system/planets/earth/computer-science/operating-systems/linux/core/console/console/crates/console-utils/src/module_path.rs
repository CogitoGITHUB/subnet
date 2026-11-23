#[doc(hidden)]
pub mod __private {
    /// Returns the module path without the leading crate segment.
    ///
    /// For example:
    /// - `"crate::a::b"` -> `"a::b"`
    ///
    /// This looks for the first occurrence of `"::"` and returns everything after it.
    ///
    /// # Panics
    ///
    /// Panics if `full` does not contain `"::"`.
    pub const fn module_path_without_crate_impl(full: &str) -> &str {
        let mut bytes = full.as_bytes();

        while let [first, rest @ ..] = bytes {
            bytes = rest;
            if *first == b':' {
                match rest {
                    [second, rest @ ..] if *second == b':' => {
                        // SAFETY:
                        // - `full` is a valid `&str`, so `bytes` is valid UTF-8.
                        // - `:` is ASCII (1 byte), so `rest` is guaranteed to start on a UTF-8 code point boundary.
                        // - Therefore the resulting byte slice is valid UTF-8, making `from_utf8_unchecked` sound.
                        // - Also this is required as dynamically slicing a &str is not possible in stable rust yet.
                        return unsafe { str::from_utf8_unchecked(rest) };
                    }
                    _ => (),
                }
            }
        }

        panic!("expected a module path containing '::', like 'crate::module'");
    }
}

/// Expands to the current module path without the crate name.
///
/// This behaves like [`module_path!`], but removes the leading crate segment.
///
/// # Panics
///
/// Panics if invoked from the crate root, because [`module_path!`] expands to
/// just the crate name there, which does not contain `"::"`.
#[macro_export]
macro_rules! module_path_without_crate {
    () => {
        const { $crate::module_path::__private::module_path_without_crate_impl(module_path!()) }
    };
}

