use {
    super::*,
    crate::conf::Conf,
};

/// all the skin things used by the broot application
/// during running
pub struct AppSkin {
    /// the skin used in the focused panel
    pub focused: PanelSkin,

    /// the skin used in unfocused panels
    pub unfocused: PanelSkin,
}

impl AppSkin {
    pub fn new(
        _conf: &Conf,
        no_style: bool,
    ) -> Self {
        if no_style {
            Self {
                focused: PanelSkin::new(StyleMap::no_term()),
                unfocused: PanelSkin::new(StyleMap::no_term()),
            }
        } else {
            let StyleMaps { focused, unfocused } = StyleMaps::create();
            Self {
                focused: PanelSkin::new(focused),
                unfocused: PanelSkin::new(unfocused),
            }
        }
    }
}
