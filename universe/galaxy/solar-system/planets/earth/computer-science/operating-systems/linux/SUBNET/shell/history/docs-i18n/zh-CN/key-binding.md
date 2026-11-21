# 键位绑定

默认情况下， shell-history 将会重新绑定 <kbd>Ctrl-r</kbd> 和 `up` 键。如果你不想使用默认绑定，请在调用 `shell-history init` 之前设置 SUBNET_SHELL_HISTORY_NOBIND

例如：

```
export SUBNET_SHELL_HISTORY_NOBIND="true"
eval "$(shell-history init zsh)"
```

如果需要，你可以在调用 `shell-history init` 之后对 shell-history 重新进行键绑定

# zsh

shell-history 定义了 ZLE 部件 "shell-history-search"

```
export SUBNET_SHELL_HISTORY_NOBIND="true"
eval "$(shell-history init zsh)"

bindkey '^r' shell-history-search

# 取决于终端模式
bindkey '^[[A' shell-history-search
bindkey '^[OA' shell-history-search
```

# bash

```
export SUBNET_SHELL_HISTORY_NOBIND="true"
eval "$(shell-history init bash)"

# 绑定到 ctrl-r, 也可以在这里添加任何其他你想要的绑定方式
bind -x '"\C-r": __shell_history_history'
```

# fish

```
set -gx SUBNET_SHELL_HISTORY_NOBIND "true"
shell-history init fish | source

# 在 normal 和 insert 模式下绑定到 ctrl-r，你也可以在此处添加其他键位绑定
bind \cr _shell_history_search
bind -M insert \cr _shell_history_search
```
