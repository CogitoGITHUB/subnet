#[cfg(not(test))]
use std::env;
use std::ffi::OsString;

#[derive(Default)]
pub struct Env<'a> {
    /// A `HashMap` of environment variable mocks

    #[cfg(not(test))]
    _marker: std::marker::PhantomData<&'a ()>,
}

#[cfg_attr(not(test), allow(clippy::needless_lifetimes))]
impl<'a> Env<'a> {
    // Retrieves a environment variable from the os or from a table if in testing mode

    #[cfg(not(test))]
    #[inline]
    pub fn get_env<K: AsRef<str>>(&self, key: K) -> Option<String> {
        env::var(key.as_ref()).ok()
    }

    // Retrieves a environment variable from the os or from a table if in testing mode (os version)

    #[cfg(not(test))]
    #[inline]
    pub fn get_env_os<K: AsRef<str>>(&self, key: K) -> Option<OsString> {
        env::var_os(key.as_ref())
    }

}
