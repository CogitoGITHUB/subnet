use std::{
    borrow::Cow,
    path::{Path, PathBuf},
};

/// Strip any trailing slashes from a non-root path. This is required in some contexts, for example
/// for the `PWD` environment variable.
pub fn strip_trailing_slash(path: &Path) -> Cow<'_, Path> {
    if has_trailing_slash(path) {
        // If there are, the safest thing to do is have Rust parse the path for us and build it
        // again. This will correctly handle a root directory, but it won't add the trailing slash.
        let mut out = PathBuf::with_capacity(path.as_os_str().len());
        out.extend(path.components());
        Cow::Owned(out)
    } else {
        // The path is safe and doesn't contain any trailing slashes.
        Cow::Borrowed(path)
    }
}

/// `true` if the path has a trailing slash, including if it's the root directory.
#[cfg(windows)]
pub fn has_trailing_slash(path: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;

    let last = path.as_os_str().encode_wide().last();
    last == Some(b'\\' as u16) || last == Some(b'/' as u16)
}

/// `true` if the path has a trailing slash, including if it's the root directory.
#[cfg(unix)]
pub fn has_trailing_slash(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;

    let last = path.as_os_str().as_bytes().last();
    last == Some(&b'/')
}

/// `true` if the path has a trailing slash, including if it's the root directory.
#[cfg(target_arch = "wasm32")]
pub fn has_trailing_slash(path: &Path) -> bool {
    // in the web paths are often just URLs, they are separated by forward slashes
    path.to_str().is_some_and(|s| s.ends_with('/'))
}

