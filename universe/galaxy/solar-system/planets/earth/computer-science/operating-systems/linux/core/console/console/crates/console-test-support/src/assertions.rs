use console_utils::container::Container;
use std::{borrow::Borrow, fmt::Debug};

/// Assert that a haystack contains the given needle.
///
/// Uses the [`Container`] abstraction so it works with slices, vectors, sets,
/// maps (by key), strings, and ranges.
/// The error message includes both the container and the item for quick debugging.
///
/// # Panics
///
/// Panics if `haystack.contains(needle)` returns false.
#[track_caller]
pub fn assert_contains<H, N>(needle: N, haystack: H)
where
    H: Container + Debug,
    N: Borrow<H::Item>,
    H::Item: Debug,
{
    let item = needle.borrow();

    assert!(
        haystack.contains(item),
        "{haystack:?} does not contain {item:?}"
    );
}

/// Assert that a haystack does not contain the given needle.
///
/// Uses the [`Container`] abstraction so it works with slices, vectors, sets,
/// maps (by key), strings, and ranges.
/// The error message includes both the container and the item for quick debugging.
///
/// # Panics
///
/// Panics if `haystack.contains(needle)` returns true.
#[track_caller]
pub fn assert_contains_not<H, N>(needle: N, haystack: H)
where
    H: Container + Debug,
    N: Borrow<H::Item>,
    H::Item: Debug,
{
    let item = needle.borrow();

    assert!(!haystack.contains(item), "{haystack:?} contains {item:?}");
}

