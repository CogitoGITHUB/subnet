if command -sq shell-jump
    shell-jump init fish | source
else
    echo 'shell-jump: command not found, please install it from https://github.com/ajeetdsouza/zoxide'
end
