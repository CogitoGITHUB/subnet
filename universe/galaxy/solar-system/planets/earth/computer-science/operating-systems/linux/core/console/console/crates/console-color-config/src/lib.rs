mod roles;
mod color_config;
mod matching_brackets_style;
mod shell_style;
mod shape_color;
mod style_computer;
mod text_style;

pub use color_config::*;
pub use matching_brackets_style::*;
pub use shell_style::*;
pub use shape_color::*;
pub use style_computer::*;
pub use text_style::*;

#[cfg(test)]
#[macro_use]
extern crate console_test_support;

#[cfg(test)]
use console_test_support::harness::main;
