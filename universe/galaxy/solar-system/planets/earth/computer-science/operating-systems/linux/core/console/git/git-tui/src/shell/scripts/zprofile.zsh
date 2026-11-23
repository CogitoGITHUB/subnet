ZDOTDIR=$GIT_TUI_USER_ZDOTDIR
[[ -f $ZDOTDIR/.zprofile ]] && builtin source $ZDOTDIR/.zprofile
ZDOTDIR=$__git_tui_inject_dir
