# Built-in default config, evaluated before the user config.nu.
# Holds the history hook and the CTRL-R / Up keybindings,
# so history works out of the box with no setup.
# Source this in your ~/.config/nushell/config.nu
# minimum supported version = 0.93.0
module compat {
  export def --wrapped "random uuid -v 7" [...rest] { console history uuid }
}
use (if not (
    (version).major > 0 or
    (version).minor >= 103
) { "compat" }) *

if 'CONSOLE_SHELL_HISTORY_SESSION' not-in $env or ('CONSOLE_SHELL_HISTORY_SHLVL' not-in $env) or ($env.CONSOLE_SHELL_HISTORY_SHLVL != ($env.SHLVL? | default "")) {
    $env.CONSOLE_SHELL_HISTORY_SESSION = (random uuid -v 7 | str replace -a "-" "")
    $env.CONSOLE_SHELL_HISTORY_SHLVL = ($env.SHLVL? | default "")
}
hide-env -i CONSOLE_SHELL_HISTORY_HISTORY_ID

def _shell_history_osc133_command_executed [] {
    if 'CONSOLE_SHELL_HISTORY_PTY_PROXY_ACTIVE' not-in $env {
        return
    }
    if 'CONSOLE_SHELL_HISTORY_HISTORY_ID' not-in $env or ($env.CONSOLE_SHELL_HISTORY_HISTORY_ID | is-empty) {
        return
    }

    print -n $"(char -u '1b')]133;C(char bel)"
}

def _shell_history_osc133_command_finished [exit_code: int] {
    if 'CONSOLE_SHELL_HISTORY_PTY_PROXY_ACTIVE' not-in $env {
        return
    }
    if 'CONSOLE_SHELL_HISTORY_HISTORY_ID' not-in $env or ($env.CONSOLE_SHELL_HISTORY_HISTORY_ID | is-empty) {
        return
    }

    print -n $"(char -u '1b')]133;D;($exit_code);history_id=($env.CONSOLE_SHELL_HISTORY_HISTORY_ID);session_id=($env.CONSOLE_SHELL_HISTORY_SESSION)(char bel)"
}

# Magic token to make sure we don't record commands run by keybindings
let CONSOLE_SHELL_HISTORY_KEYBINDING_TOKEN = $"# (random uuid)"

let _shell_history_pre_execution = {||
    if ($nu | get history-enabled?) == false {
        return
    }
    let cmd = (commandline)
    if ($cmd | is-empty) {
        return
    }
    if not ($cmd | str starts-with $CONSOLE_SHELL_HISTORY_KEYBINDING_TOKEN) {
        $env.CONSOLE_SHELL_HISTORY_HISTORY_ID = (console history history start -- $cmd | complete | get stdout | str trim)
        _shell_history_osc133_command_executed
    }
}

let _shell_history_pre_prompt = {||
    let last_exit = $env.LAST_EXIT_CODE
    if 'CONSOLE_SHELL_HISTORY_HISTORY_ID' not-in $env {
        return
    }
    _shell_history_osc133_command_finished $last_exit
    with-env { CONSOLE_SHELL_HISTORY_LOG: error } {
        if (version).minor >= 104 or (version).major > 0 {
            job spawn {
                ^console history history end $'--exit=($env.LAST_EXIT_CODE)' -- $env.CONSOLE_SHELL_HISTORY_HISTORY_ID | complete
            } | ignore
        } else {
            do { console history history end $'--exit=($last_exit)' -- $env.CONSOLE_SHELL_HISTORY_HISTORY_ID } | complete
        }

    }
    hide-env -i CONSOLE_SHELL_HISTORY_HISTORY_ID
}

def _shell_history_search_cmd [...flags: string] {
    if (version).minor >= 106 or (version).major > 0 {
        [
            $CONSOLE_SHELL_HISTORY_KEYBINDING_TOKEN,
            ([
                `with-env { CONSOLE_SHELL_HISTORY_LOG: error, CONSOLE_SHELL_HISTORY_QUERY: (commandline), CONSOLE_SHELL_HISTORY_SHELL: nu } {`,
                    ([
                        'let output = (run-external console history search',
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
            $CONSOLE_SHELL_HISTORY_KEYBINDING_TOKEN,
            ([
                `with-env { CONSOLE_SHELL_HISTORY_LOG: error, CONSOLE_SHELL_HISTORY_QUERY: (commandline) } {`,
                    'commandline edit',
                    '(run-external console history search',
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

$env.config = ($env.config | upsert keybindings (
  $env.config.keybindings | append {
    name: shell_history
    modifier: control
    keycode: char_r
    mode: [emacs, vi_normal, vi_insert]
    event: {
      send: executehostcommand
      cmd: (_shell_history_search_cmd)
    }
  }
))

$env.config = ($env.config | upsert keybindings (
  $env.config.keybindings | append {
    name: shell_history_up
    modifier: none
    keycode: up
    mode: [emacs, vi_normal, vi_insert]
    event: {
      until: [
        { send: menuup }
        {
          send: executehostcommand
          cmd: (_shell_history_search_cmd '--shell-up-key-binding')
        }
      ]
    }
  }
))

# Start the local history server and register, once per shell.
if (version).minor >= 104 or (version).major > 0 {
    job spawn { ^console history ensure | complete } | ignore
}
