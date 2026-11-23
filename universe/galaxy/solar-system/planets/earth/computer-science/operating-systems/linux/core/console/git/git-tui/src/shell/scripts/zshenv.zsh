# git-tui terminal: $ZDOTDIR points here. Each startup file loads the
# user's own copy, then hands back to this directory.
__git_tui_inject_dir=$ZDOTDIR
ZDOTDIR=${GIT_TUI_USER_ZDOTDIR:-$HOME}
[[ -f $ZDOTDIR/.zshenv ]] && builtin source $ZDOTDIR/.zshenv
GIT_TUI_USER_ZDOTDIR=$ZDOTDIR
ZDOTDIR=$__git_tui_inject_dir
