if [[ -n ${GIT_TUI_LOGIN-} ]]; then
  [[ -r /etc/profile ]] && builtin source /etc/profile
  if [[ -r ~/.bash_profile ]]; then builtin source ~/.bash_profile
  elif [[ -r ~/.bash_login ]]; then builtin source ~/.bash_login
  elif [[ -r ~/.profile ]]; then builtin source ~/.profile
  fi
else
  [[ -r ~/.bashrc ]] && builtin source ~/.bashrc
fi
__git_tui_nonce=$GIT_TUI_NONCE
builtin unset GIT_TUI_NONCE GIT_TUI_LOGIN
__git_tui_prompt() {
  local s=$?
  builtin printf '\033]7777;git-tui;%s;C;%s\007' "$__git_tui_nonce" "$PWD" >/dev/tty
  return $s
}
PROMPT_COMMAND="__git_tui_prompt${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
