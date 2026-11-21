use shell_history_client::settings::Tmux;
use shell_history_dotfiles::store::{AliasStore, var::VarStore};

pub fn init_static(disable_up_arrow: bool, disable_ctrl_r: bool, _tmux: &Tmux) {
    let base = include_str!("../../../shell/shell-history.ps1");

    let (bind_ctrl_r, bind_up_arrow) = if std::env::var("SUBNET_SHELL_HISTORY_NOBIND").is_ok() {
        (false, false)
    } else {
        (!disable_ctrl_r, !disable_up_arrow)
    };

    // TODO: tmux popup for Powershell
    println!("{base}");
    println!(
        "Enable-RunShellHistorySearchKeys -CtrlR {} -UpArrow {}",
        ps_bool(bind_ctrl_r),
        ps_bool(bind_up_arrow)
    );
}

pub async fn init(
    aliases: AliasStore,
    vars: VarStore,
    disable_up_arrow: bool,
    disable_ctrl_r: bool,
    tmux: &Tmux,
) -> eyre::Result<()> {
    init_static(disable_up_arrow, disable_ctrl_r, tmux);

    let aliases = shell_history_dotfiles::shell::powershell::alias_config(&aliases).await;
    let vars = shell_history_dotfiles::shell::powershell::var_config(&vars).await;

    println!("{aliases}");
    println!("{vars}");

    Ok(())
}

fn ps_bool(value: bool) -> &'static str {
    if value { "$true" } else { "$false" }
}
