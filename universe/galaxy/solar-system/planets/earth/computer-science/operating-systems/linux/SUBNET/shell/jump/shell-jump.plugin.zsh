if (( $+commands[shell-jump] )); then
  eval "$(shell-jump init zsh)"
else
  echo 'shell-jump: command not found, please install it from https://github.com/ajeetdsouza/zoxide'
fi
