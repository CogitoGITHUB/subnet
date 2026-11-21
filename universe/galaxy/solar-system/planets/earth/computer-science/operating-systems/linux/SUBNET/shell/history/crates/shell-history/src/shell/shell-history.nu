# Source this in your ~/.config/nushell/config.nu
# minimum supported version = 0.93.0
module compat {
  export def --wrapped "random uuid -v 7" [...rest] { shell-history uuid }
}
use (if not (
    (version).major > 0 or
    (version).minor >= 103
) { "compat" }) *

if 'SUBNET_SHELL_HISTORY_SESSION' not-in $env or ('SUBNET_SHELL_HISTORY_SHLVL' not-in $env) or ($env.SUBNET_SHELL_HISTORY_SHLVL != ($env.SHLVL? | default "")) {
    $env.SUBNET_SHELL_HISTORY_SESSION = (random uuid -v 7 | str replace -a "-" "")
    $env.SUBNET_SHELL_HISTORY_SHLVL = ($env.SHLVL? | default "")
}
hide-env -i SUBNET_SHELL_HISTORY_HISTORY_ID

def _shell_history_osc133_command_executed [] {
    if 'SUBNET_SHELL_HISTORY_PTY_PROXY_ACTIVE' not-in $env {
        return
    }
    if 'SUBNET_SHELL_HISTORY_HISTORY_ID' not-in $env or ($env.SUBNET_SHELL_HISTORY_HISTORY_ID | is-empty) {
        return
    }

    print -n $"(char -u '1b')]133;C(char bel)"
}

def _shell_history_osc133_command_finished [exit_code: int] {
    if 'SUBNET_SHELL_HISTORY_PTY_PROXY_ACTIVE' not-in $env {
        return
    }
    if 'SUBNET_SHELL_HISTORY_HISTORY_ID' not-in $env or ($env.SUBNET_SHELL_HISTORY_HISTORY_ID | is-empty) {
        return
    }

    print -n $"(char -u '1b')]133;D;($exit_code);history_id=($env.SUBNET_SHELL_HISTORY_HISTORY_ID);session_id=($env.SUBNET_SHELL_HISTORY_SESSION)(char bel)"
}

# Magic token to make sure we don't record commands run by keybindings
let SUBNET_SHELL_HISTORY_KEYBINDING_TOKEN = $"# (random uuid)"

let _shell_history_pre_execution = {||
    if ($nu | get history-enabled?) == false {
        return
    }
    let cmd = (commandline)
    if ($cmd | is-empty) {
        return
    }
    if not ($cmd | str starts-with $SUBNET_SHELL_HISTORY_KEYBINDING_TOKEN) {
        $env.SUBNET_SHELL_HISTORY_HISTORY_ID = (shell-history history start -- $cmd | complete | get stdout | str trim)
        _shell_history_osc133_command_executed
    }
}

let _shell_history_pre_prompt = {||
    let last_exit = $env.LAST_EXIT_CODE
    if 'SUBNET_SHELL_HISTORY_HISTORY_ID' not-in $env {
        return
    }
    _shell_history_osc133_command_finished $last_exit
    with-env { SUBNET_SHELL_HISTORY_LOG: error } {
        if (version).minor >= 104 or (version).major > 0 {
            job spawn {
                ^shell-history history end $'--exit=($env.LAST_EXIT_CODE)' -- $env.SUBNET_SHELL_HISTORY_HISTORY_ID | complete
            } | ignore
        } else {
            do { shell-history history end $'--exit=($last_exit)' -- $env.SUBNET_SHELL_HISTORY_HISTORY_ID } | complete
        }

    }
    hide-env -i SUBNET_SHELL_HISTORY_HISTORY_ID
}

def _shell_history_search_cmd [...flags: string] {
    if (version).minor >= 106 or (version).major > 0 {
        [
            $SUBNET_SHELL_HISTORY_KEYBINDING_TOKEN,
            ([
                `with-env { SUBNET_SHELL_HISTORY_LOG: error, SUBNET_SHELL_HISTORY_QUERY: (commandline), SUBNET_SHELL_HISTORY_SHELL: nu } {`,
                    ([
                        'let output = (run-external shell-history search',
                        ($flags | append [--interactive] | each {|e| $'"($e)"'}),
                        'e>| str trim)',
                    ] | flatten | str join ' '),
                    'if ($output | str starts-with "__shell_history_accept__:") {',
                    'commandline edit --accept ($output | str replace "__shell_history_accept__:" "")',
                    '} else {',
                    'commandline edit $output',
                    '}',
                `}`,
            ] | flatten | str join "\n"),
        ]
    } else {
        [
            $SUBNET_SHELL_HISTORY_KEYBINDING_TOKEN,
            ([
                `with-env { SUBNET_SHELL_HISTORY_LOG: error, SUBNET_SHELL_HISTORY_QUERY: (commandline) } {`,
                    'commandline edit',
                    '(run-external shell-history search',
                        ($flags | append [--interactive] | each {|e| $'"($e)"'}),
                    ' e>| str trim)',
                `}`,
            ] | flatten | str join ' '),
        ]
    } | str join "\n"
}

$env.config = ($env | default {} config).config
$env.config = ($env.config | default {} hooks)
$env.config = (
    $env.config | upsert hooks (
        $env.config.hooks
        | upsert pre_execution (
            $env.config.hooks | get pre_execution? | default [] | append $_shell_history_pre_execution)
        | upsert pre_prompt (
            $env.config.hooks | get pre_prompt? | default [] | append $_shell_history_pre_prompt)
    )
)

$env.config = ($env.config | default [] keybindings)
