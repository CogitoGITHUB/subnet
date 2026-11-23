ZDOTDIR=$GIT_TUI_USER_ZDOTDIR
[[ -f $ZDOTDIR/.zshrc ]] && builtin source $ZDOTDIR/.zshrc
ZDOTDIR=$__git_tui_inject_dir
builtin source $__git_tui_inject_dir/git-tui.zsh
