typeset -g __git_tui_nonce=$GIT_TUI_NONCE __git_tui_first=1
unset GIT_TUI_NONCE
__git_tui_precmd() {
  local s=$?
  if (( __git_tui_first )); then
    __git_tui_first=0
    ZDOTDIR=$GIT_TUI_USER_ZDOTDIR
    # A system /etc/zshrc runs while ZDOTDIR is still our directory and may
    # derive HISTFILE from it: point it at the user's directory instead.
    if [[ -n ${HISTFILE-} && $HISTFILE == $__git_tui_inject_dir/* ]]; then
      HISTFILE=$GIT_TUI_USER_ZDOTDIR/${HISTFILE:t}
      fc -R "$HISTFILE" 2>/dev/null
    fi
    unset GIT_TUI_USER_ZDOTDIR
  fi
  builtin printf '\033]7777;git-tui;%s;C;%s\007' "$__git_tui_nonce" "$PWD" >/dev/tty
  return $s
}
precmd_functions=(__git_tui_precmd $precmd_functions)
