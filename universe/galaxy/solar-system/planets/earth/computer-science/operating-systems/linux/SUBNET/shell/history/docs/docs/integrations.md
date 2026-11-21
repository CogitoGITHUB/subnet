# Integrations

This page covers integrations with shell plugins and tools. For information about how shell-history's shell hooks work and troubleshooting embedded terminals (IDEs, AI coding assistants, etc.), see [Shell Integration and Interoperability](guide/shell-integration.md).

## zsh-autosuggestions

shell-history automatically adds itself as an [autosuggest strategy](https://github.com/zsh-users/zsh-autosuggestions#suggestion-strategy).

If you'd like to override this, add your own config after `"$(shell-history init zsh)"` in your `.zshrc`.

## zsh-vi-mode

If you are using [Zsh Vi Mode](https://github.com/jeffreytse/zsh-vi-mode), you may want to add the following to your `.zshrc` to prevent overriding the default shell-history binds:

```shell
# Append a command directly (after sourcing zvm)
zvm_after_init_commands+=(
  'eval "$(shell-history init zsh)"'
)
```

## ble.sh auto-complete (Bash)

If ble.sh is available when shell-history's integration is loaded in Bash, shell-history automatically defines and registers an auto-complete source for the autosuggestion feature of ble.sh.

If you'd like to change the behavior, please overwrite the shell function `ble/complete/auto-complete/source:shell-history-history` after `eval "$(shell-history init bash)"` in your `.bashrc`.

If you would not like shell-history's auto-complete source, please add the following setting after `eval "$(shell-history init bash)"` in your `.bashrc`:

```shell
# bashrc (after eval "$(shell-history init bash)")

ble/util/import/eval-after-load core-complete '
  ble/array#remove _ble_complete_auto_source shell-history-history'
```

## Embedded Terminals and IDEs

shell-history may not work out of the box in embedded terminals found in IDEs (PyCharm, VS Code, etc.) or AI coding assistants (Claude Code, etc.). This is because these tools often start non-interactive shells that don't source your shell configuration.

For solutions and workarounds, see [Shell Integration and Interoperability](guide/shell-integration.md).
