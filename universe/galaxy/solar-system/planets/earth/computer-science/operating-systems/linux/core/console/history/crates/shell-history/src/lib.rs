#![warn(clippy::pedantic, clippy::nursery)]
#![allow(clippy::use_self, clippy::missing_const_for_fn)] // not 100% reliable

use clap::Parser;
use clap::builder::Styles;
use clap::builder::styling::{Effects, Style};
use eyre::Result;

use command::RunShellHistoryCmd;

pub mod command;

#[cfg(feature = "sync")]
pub mod print_error;
#[cfg(feature = "sync")]
pub mod sync;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const SHA: &str = env!("GIT_HASH");

const LONG_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " (", env!("GIT_HASH"), ")");

static HELP_TEMPLATE: &str = "\
{before-help}{name} {version}
{author}
{about}

{usage-heading}
  {usage}

{all-args}{after-help}";

const STYLES: Styles = Styles::styled()
    .header(Style::new().effects(Effects::BOLD))
    .usage(Style::new().effects(Effects::BOLD))
    .literal(Style::new().effects(Effects::BOLD))
    .placeholder(Style::new().effects(Effects::DIMMED));

/// Magical shell history
#[derive(Parser)]
#[command(
    author = "console",
    version = VERSION,
    long_version = LONG_VERSION,
    help_template(HELP_TEMPLATE),
    styles = STYLES,
)]
struct RunShellHistory {
    #[command(subcommand)]
    shell_history: RunShellHistoryCmd,
}

impl RunShellHistory {
    fn run(self) -> Result<()> {
        self.shell_history.run()
    }
}

pub fn run(argv: Vec<std::ffi::OsString>) -> Result<()> {
    RunShellHistory::parse_from(argv).run()
}

mod style;
