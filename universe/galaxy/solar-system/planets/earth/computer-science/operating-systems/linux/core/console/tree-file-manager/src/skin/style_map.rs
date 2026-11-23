/// Defines the StyleMap structure with its default value.
///
/// A style_map is a collection of termimad compound_style. It is
/// either defined for the focused panel state or the unfocused
/// one (there are thus two instances in the application).
/// All colors come from the four style roles (see roles.rs).
use {
    super::{
        roles::{bgc, dm, er, tx},
        *,
    },
    crate::errors::ProgramError,
    crokey::crossterm::{
        QueueableCommand,
        style::{
            Attribute::*,
            Attributes,
            Color,
            Color::*,
            SetBackgroundColor,
        },
    },
    std::{
        fmt,
        io::Write,
    },
    termimad::CompoundStyle,
};

macro_rules! StyleMap {
    (
        $(
            $name:ident: $fg:expr, $bg:expr, [$($attr:expr)*] $( / $fgu:expr, $bgu:expr , [$($attru:expr)*] )*
        )*
    ) => {
        /// a struct whose fields are
        /// - a boolean telling whether it is a no-style map
        /// - the styles to apply to various parts/cases
        pub struct StyleMap {
            styled: bool,
            $(pub $name: CompoundStyle,)*
        }
        /// a set of two style_maps: one for the focused panel
        /// and one for the other panels
        pub struct StyleMaps {
            pub focused: StyleMap,
            pub unfocused: StyleMap,
        }
        impl StyleMap {
            /// build a skin without any terminal control
            /// character (for file output)
            pub fn no_term() -> Self {
                Self {
                    styled: false,
                    $($name: CompoundStyle::default(),)*
                }
            }
            /// ensures the "default" entry is used as base
            /// for all other entries
            fn diffuse_default(&mut self) {
                $(
                    let mut base = self.default.clone();
                    base.overwrite_with(&self.$name);
                    self.$name = base;
                )*
            }
        }
        impl StyleMaps {
            pub fn create() -> Self {
                let mut focused = StyleMap {
                    styled: true,
                    $($name: CompoundStyle::new(
                        $fg,
                        $bg,
                        Attributes::from(vec![$($attr),*].as_slice()),
                    ),)*
                };
                focused.diffuse_default();
                let mut unfocused = StyleMap {
                    styled: true,
                    $($name: CompoundStyle::default(),)*
                };
                $(
                    unfocused.$name = CompoundStyle::new(
                        $fg,
                        $bg,
                        Attributes::from(vec![$($attr),*].as_slice()),
                    );
                    $(
                        unfocused.$name = CompoundStyle::new(
                            $fgu,
                            $bgu,
                            Attributes::from(vec![$($attru),*].as_slice()),
                        );
                    )*
                )*
                unfocused.diffuse_default();
                Self {
                    focused,
                    unfocused,
                }
            }
        }
        impl Clone for StyleMap {
            fn clone(&self) -> Self {
                Self {
                    styled: self.styled,
                    $($name: self.$name.clone(),)*
                }
            }
        }
    }
}

impl StyleMap {
    pub fn queue_reset<W: Write>(
        &self,
        f: &mut W,
    ) -> Result<(), ProgramError> {
        if self.styled {
            f.queue(SetBackgroundColor(Color::Reset))?;
        }
        Ok(())
    }
    pub fn good_to_bad_color(
        &self,
        value: f64,
    ) -> Color {
        debug_assert!((0.0..=1.0).contains(&value));
        const N: usize = 10;
        let idx = (value * N as f64) as usize;
        let cs = match idx {
            0 => &self.good_to_bad_0,
            1 => &self.good_to_bad_1,
            2 => &self.good_to_bad_2,
            3 => &self.good_to_bad_3,
            4 => &self.good_to_bad_4,
            5 => &self.good_to_bad_5,
            6 => &self.good_to_bad_6,
            7 => &self.good_to_bad_7,
            8 => &self.good_to_bad_8,
            _ => &self.good_to_bad_9,
        };
        cs.object_style.foreground_color.unwrap_or(crate::skin::roles::txc())
    }
}

// Styles defined as
//    name: forecolor, backcolor, [attributes]
// The optional part after a slash is the style for unfocused panels
// (if missing the style is the same than for focused panels)
StyleMap! {
    default: tx(), bgc(), [] / dm(), bgc(), []
    tree: dm(), None, []
    parent: dm(), None, []
    file: tx(), None, [] / dm(), None, []
    directory: tx(), None, [Bold] / dm(), None, []
    exe: tx(), None, []
    link: tx(), None, []
    pruning: dm(), None, [Italic]
    perm__: dm(), None, []
    perm_r: dm(), None, []
    perm_w: dm(), None, []
    perm_x: dm(), None, []
    owner: dm(), None, []
    group: dm(), None, []
    count: dm(), None, []
    dates: dm(), None, []
    sparse: dm(), None, []
    content_extract: dm(), None, []
    content_match: tx(), None, [Bold]
    device_id_major: dm(), None, []
    device_id_sep: dm(), None, []
    device_id_minor: dm(), None, []
    git_branch: tx(), None, []
    git_insertions: tx(), None, []
    git_deletions: er(), None, []
    git_status_current: dm(), None, []
    git_status_modified: tx(), None, []
    git_status_new: tx(), None, [Bold]
    git_status_ignored: dm(), None, []
    git_status_conflicted: er(), None, []
    git_status_other: er(), None, []
    selected_line: None, dm(), []
    char_match: tx(), None, [Bold]
    file_error: er(), None, []
    flag_label: dm(), bgc(), []
    flag_value: tx(), bgc(), [Bold]
    input: tx(), bgc(), [] / dm(), None, []
    status_error: bgc(), er(), []
    status_job: tx(), dm(), []
    status_normal: tx(), dm(), [] / dm(), bgc(), []
    status_italic: tx(), dm(), [Italic] / dm(), bgc(), [Italic]
    status_bold: tx(), dm(), [Bold] / dm(), bgc(), [Bold]
    status_code: tx(), dm(), [] / dm(), bgc(), []
    status_ellipsis: tx(), dm(), [] / dm(), bgc(), []
    purpose_normal: tx(), bgc(), []
    purpose_italic: tx(), bgc(), [Italic]
    purpose_bold: tx(), bgc(), [Bold]
    purpose_ellipsis: tx(), bgc(), []
    scrollbar_track: dm(), None, []
    scrollbar_thumb: tx(), None, [] / dm(), None, []
    help_paragraph: tx(), None, []
    help_bold: tx(), None, [Bold]
    help_italic: tx(), None, [Italic]
    help_code: tx(), dm(), []
    help_headers: tx(), None, [Bold]
    help_table_border: dm(), None, []
    preview: tx(), bgc(), [] / dm(), bgc(), []
    preview_title: tx(), bgc(), [Bold] / dm(), bgc(), []
    preview_line_number: dm(), None, []
    preview_separator: dm(), None, []
    preview_match: None, dm(), []
    hex_null: dm(), None, []
    hex_ascii_graphic: tx(), None, []
    hex_ascii_whitespace: dm(), None, []
    hex_ascii_other: tx(), None, []
    hex_non_ascii: er(), None, []
    staging_area_title: tx(), bgc(), [Bold] / dm(), bgc(), []
    mode_command_mark: bgc(), tx(), [Bold]
    good_to_bad_0: tx(), None, []
    good_to_bad_1: tx(), None, []
    good_to_bad_2: tx(), None, []
    good_to_bad_3: tx(), None, []
    good_to_bad_4: tx(), None, []
    good_to_bad_5: dm(), None, []
    good_to_bad_6: dm(), None, []
    good_to_bad_7: dm(), None, []
    good_to_bad_8: er(), None, []
    good_to_bad_9: er(), None, []
}

impl fmt::Debug for StyleMap {
    fn fmt(
        &self,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        write!(f, "Skin")
    }
}
