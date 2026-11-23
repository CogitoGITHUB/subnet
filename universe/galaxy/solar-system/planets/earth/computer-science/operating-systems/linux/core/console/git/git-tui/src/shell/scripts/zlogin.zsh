ZDOTDIR=$GIT_TUI_USER_ZDOTDIR
[[ -f $ZDOTDIR/.zlogin ]] && builtin source $ZDOTDIR/.zlogin
ZDOTDIR=$__git_tui_inject_dir
