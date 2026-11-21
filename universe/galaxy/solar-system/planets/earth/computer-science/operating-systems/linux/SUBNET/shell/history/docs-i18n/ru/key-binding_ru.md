# Key binding

По умолчанию, shell-history будет переназначать <kbd>Ctrl-r</kbd> и клавишу 'стрелка вверх'.
Если вы не хотите этого, установите параметр SUBNET_SHELL_HISTORY_NOBIND прежде чем вызывать `shell-history init`

Например,

```
export SUBNET_SHELL_HISTORY_NOBIND="true"
eval "$(shell-history init zsh)"
```

Таким образом вы можете разрешить переназначение клавиш shell-history, если это необходимо.
Делайте это до инициализирующего вызова.

# zsh

shell-history устанавливает виджет ZLE "shell-history-search"

```
export SUBNET_SHELL_HISTORY_NOBIND="true"
eval "$(shell-history init zsh)"

bindkey '^r' shell-history-search

# зависит от режима терминала
bindkey '^[[A' shell-history-search
bindkey '^[OA' shell-history-search
```

# bash

```
export SUBNET_SHELL_HISTORY_NOBIND="true"
eval "$(shell-history init bash)"

# Переопределите  ctrl-r, и любые другие сочетания горячих клавиш тут
bind -x '"\C-r": __shell_history_history'
```
