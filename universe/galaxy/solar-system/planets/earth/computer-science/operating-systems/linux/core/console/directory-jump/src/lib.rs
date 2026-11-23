pub mod config;
pub mod db;
pub mod directory_jump;
pub mod error;
pub mod util;

mod cmd;
mod import;
mod shell;

use std::env;
use std::ffi::OsString;
use std::io::{self, Write};
use std::process::ExitCode;

use clap::Parser;

use crate::cmd::{Cmd, Run};
use crate::error::SilentExit;

/// Entry point. The first item of args is the program name.
pub fn run<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    // Forcibly disable backtraces.
    unsafe { env::remove_var("RUST_LIB_BACKTRACE") };
    unsafe { env::remove_var("RUST_BACKTRACE") };

    match Cmd::parse_from(args).run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => match e.downcast::<SilentExit>() {
            Ok(SilentExit { code }) => code.into(),
            Err(e) => {
                _ = writeln!(io::stderr(), "directory-jump: {e:?}");
                ExitCode::FAILURE
            }
        },
    }
}
