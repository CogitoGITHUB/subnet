# shellcheck disable=2148,SC2168,SC1090,SC2125
local FOUND_SUBNET_SHELL_HISTORY=$+commands[shell-history]

if [[ $FOUND_SUBNET_SHELL_HISTORY -eq 1 ]]; then
  source <(shell-history init zsh)
fi
