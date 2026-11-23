use clap::Parser;
use eyre::Result;
use shell_history_client::settings::Settings;

#[derive(Parser, Debug)]
pub struct Cmd {
    /// Do not bind CTRL-R
    #[clap(long)]
    disable_ctrl_r: bool,

    /// Do not bind the Up Arrow key
    #[clap(long)]
    disable_up_arrow: bool,
}

const HOOK: &str = include_str!("../../shell/shell-history.nu");

const BIND_CTRL_R: &str = r"
$env.config = ($env.config | upsert keybindings (
  $env.config.keybindings | append {
    name: shell_history
    modifier: control
    keycode: char_r
    mode: [emacs, vi_normal, vi_insert]
    event: {
      send: executehostcommand
      cmd: (_shell_history_search_cmd)
    }
  }
))
";

const BIND_UP_ARROW: &str = r"
$env.config = ($env.config | upsert keybindings (
  $env.config.keybindings | append {
    name: shell_history
    modifier: none
    keycode: up
    mode: [emacs, vi_normal, vi_insert]
    event: {
      until: [
        { send: menuup }
        {
          send: executehostcommand
          cmd: (_shell_history_search_cmd)
        }
      ]
    }
  }
))
";

impl Cmd {
    pub async fn run(self, settings: &Settings) -> Result<()> {
        if !settings.paths_ok() {
            eprintln!("history paths are broken, hook disabled.");
            eprintln!("Run: console history doctor");
            return Ok(());
        }

        println!("{HOOK}");

        if std::env::var("CONSOLE_SHELL_HISTORY_NOBIND").is_err() {
            if !self.disable_ctrl_r {
                println!("{BIND_CTRL_R}");
            }
            if !self.disable_up_arrow {
                println!("{BIND_UP_ARROW}");
            }
        }
        Ok(())
    }
}
