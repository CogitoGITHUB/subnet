set -g __git_tui_nonce $GIT_TUI_NONCE
set -e GIT_TUI_NONCE
function __git_tui_cwd --on-event fish_prompt
    printf '\033]7777;git-tui;%s;C;%s\007' $__git_tui_nonce $PWD >/dev/tty
end
