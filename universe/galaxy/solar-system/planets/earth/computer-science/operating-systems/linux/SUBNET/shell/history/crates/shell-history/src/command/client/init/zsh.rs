use shell_history_client::settings::Tmux;
use shell_history_dotfiles::store::{AliasStore, var::VarStore};
use eyre::Result;

fn print_tmux_config(tmux: &Tmux) {
    if tmux.enabled {
        println!("export SUBNET_SHELL_HISTORY_TMUX_POPUP_WIDTH='{}'", tmux.width);
        println!("export SUBNET_SHELL_HISTORY_TMUX_POPUP_HEIGHT='{}'", tmux.height);
    } else {
        println!("export SUBNET_SHELL_HISTORY_TMUX_POPUP=false");
    }
}

pub fn init_static(disable_up_arrow: bool, disable_ctrl_r: bool, disable_ai: bool, tmux: &Tmux) {
    let base = include_str!("../../../shell/shell-history.zsh");

    print_tmux_config(tmux);
    println!("{base}");

    if std::env::var("SUBNET_SHELL_HISTORY_NOBIND").is_err() {
        const BIND_CTRL_R: &str = r"bindkey -M emacs '^r' shell-history-search
bindkey -M viins '^r' shell_history-search-viins
bindkey -M vicmd '/' shell_history-search";

        const BIND_UP_ARROW: &str = r"bindkey -M emacs '^[[A' shell-history-up-search
bindkey -M vicmd '^[[A' shell_history-up-search-vicmd
bindkey -M viins '^[[A' shell_history-up-search-viins
bindkey -M emacs '^[OA' shell_history-up-search
bindkey -M vicmd '^[OA' shell_history-up-search-vicmd
bindkey -M viins '^[OA' shell_history-up-search-viins
bindkey -M vicmd 'k' shell_history-up-search-vicmd";

        if !disable_ctrl_r {
            println!("{BIND_CTRL_R}");
        }
        if !disable_up_arrow {
            println!("{BIND_UP_ARROW}");
        }

        #[cfg(feature = "ai")]
        if !disable_ai {
            let bind_ai = shell_history_ai::commands::init::generate_zsh_integration();

            println!("{bind_ai}");
        }
    }
}

pub async fn init(
    aliases: AliasStore,
    vars: VarStore,
    disable_up_arrow: bool,
    disable_ctrl_r: bool,
    disable_ai: bool,
    tmux: &Tmux,
) -> Result<()> {
    init_static(disable_up_arrow, disable_ctrl_r, disable_ai, tmux);

    let aliases = shell_history_dotfiles::shell::zsh::alias_config(&aliases).await;
    let vars = shell_history_dotfiles::shell::zsh::var_config(&vars).await;

    println!("{aliases}");
    println!("{vars}");

    Ok(())
}
