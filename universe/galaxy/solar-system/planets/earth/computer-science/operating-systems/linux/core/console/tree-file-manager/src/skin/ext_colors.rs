use {
    crate::errors::InvalidSkinError,
    crokey::crossterm::style::Color,
    rustc_hash::FxHashMap,
    std::convert::TryFrom,
};

/// Extension colors are removed: the style engine is the only
/// color source. The type stays so callers keep compiling.
#[derive(Debug, Clone, Default)]
pub struct ExtColorMap {}

impl ExtColorMap {
    pub fn get(&self, _ext: &str) -> Option<Color> {
        None
    }
    pub fn set(&mut self, _ext: String, _raw: &str) -> Result<(), InvalidSkinError> {
        Ok(())
    }
}

impl TryFrom<&FxHashMap<String, String>> for ExtColorMap {
    type Error = InvalidSkinError;
    fn try_from(_raw: &FxHashMap<String, String>) -> Result<Self, Self::Error> {
        Ok(ExtColorMap::default())
    }
}
