_shell-jump() {
    local i cur prev opts cmd
    COMPREPLY=()
    if [[ "${BASH_VERSINFO[0]}" -ge 4 ]]; then
        cur="$2"
    else
        cur="${COMP_WORDS[COMP_CWORD]}"
    fi
    prev="$3"
    cmd=""
    opts=""

    for i in "${COMP_WORDS[@]:0:COMP_CWORD}"
    do
        case "${cmd},${i}" in
            ",$1")
                cmd="run__shell__jump"
                ;;
            run__shell__jump,add)
                cmd="run__shell__jump__subcmd__add"
                ;;
            run__shell__jump,edit)
                cmd="run__shell__jump__subcmd__edit"
                ;;
            run__shell__jump,import)
                cmd="run__shell__jump__subcmd__import"
                ;;
            run__shell__jump,init)
                cmd="run__shell__jump__subcmd__init"
                ;;
            run__shell__jump,query)
                cmd="run__shell__jump__subcmd__query"
                ;;
            run__shell__jump,remove)
                cmd="run__shell__jump__subcmd__remove"
                ;;
            run__shell__jump__subcmd__edit,decrement)
                cmd="run__shell__jump__subcmd__edit__subcmd__decrement"
                ;;
            run__shell__jump__subcmd__edit,delete)
                cmd="run__shell__jump__subcmd__edit__subcmd__delete"
                ;;
            run__shell__jump__subcmd__edit,increment)
                cmd="run__shell__jump__subcmd__edit__subcmd__increment"
                ;;
            run__shell__jump__subcmd__edit,reload)
                cmd="run__shell__jump__subcmd__edit__subcmd__reload"
                ;;
            run__shell__jump__subcmd__import,atuin)
                cmd="run__shell__jump__subcmd__import__subcmd__atuin"
                ;;
            run__shell__jump__subcmd__import,autojump)
                cmd="run__shell__jump__subcmd__import__subcmd__autojump"
                ;;
            run__shell__jump__subcmd__import,fasd)
                cmd="run__shell__jump__subcmd__import__subcmd__fasd"
                ;;
            run__shell__jump__subcmd__import,z)
                cmd="run__shell__jump__subcmd__import__subcmd__z"
                ;;
            run__shell__jump__subcmd__import,z.lua)
                cmd="run__shell__jump__subcmd__import__subcmd__z.lua"
                ;;
            run__shell__jump__subcmd__import,zsh-z)
                cmd="run__shell__jump__subcmd__import__subcmd__zsh__subcmd__z"
                ;;
            *)
                ;;
        esac
    done

    case "${cmd}" in
        run__shell__jump)
            opts="-h -V --help --version add edit import init query remove"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 1 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        run__subcmd__shell__subcmd__jump__subcmd__add)
            opts="-s -h -V --score --help --version <PATHS>..."
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --score)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -s)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        run__subcmd__shell__subcmd__jump__subcmd__edit)
            opts="-h -V --help --version decrement delete increment reload"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        run__subcmd__shell__subcmd__jump__subcmd__edit__subcmd__decrement)
            opts="-h -V --help --version <PATH>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        run__subcmd__shell__subcmd__jump__subcmd__edit__subcmd__delete)
            opts="-h -V --help --version <PATH>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        run__subcmd__shell__subcmd__jump__subcmd__edit__subcmd__increment)
            opts="-h -V --help --version <PATH>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        run__subcmd__shell__subcmd__jump__subcmd__edit__subcmd__reload)
            opts="-h -V --help --version"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        run__subcmd__shell__subcmd__jump__subcmd__import)
            opts="-h -V --merge --help --version atuin autojump fasd z z.lua zsh-z"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        run__subcmd__shell__subcmd__jump__subcmd__import__subcmd__atuin)
            opts="-h -V --merge --help --version"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        run__subcmd__shell__subcmd__jump__subcmd__import__subcmd__autojump)
            opts="-h -V --merge --help --version"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        run__subcmd__shell__subcmd__jump__subcmd__import__subcmd__fasd)
            opts="-h -V --merge --help --version"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        run__subcmd__shell__subcmd__jump__subcmd__import__subcmd__z)
            opts="-h -V --merge --help --version"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        run__subcmd__shell__subcmd__jump__subcmd__import__subcmd__z.lua)
            opts="-h -V --merge --help --version"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        run__subcmd__shell__subcmd__jump__subcmd__import__subcmd__zsh__subcmd__z)
            opts="-h -V --merge --help --version"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        run__subcmd__shell__subcmd__jump__subcmd__init)
            opts="-h -V --no-cmd --cmd --hook --help --version bash elvish fish nushell posix powershell tcsh xonsh zsh"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --cmd)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --hook)
                    COMPREPLY=($(compgen -W "none prompt pwd" -- "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        run__subcmd__shell__subcmd__jump__subcmd__query)
            opts="-a -i -l -s -h -V --all --interactive --list --score --exclude --base-dir --help --version [KEYWORDS]..."
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --exclude)
                    COMPREPLY=()
                    if [[ "${BASH_VERSINFO[0]}" -ge 4 ]]; then
                        compopt -o plusdirs
                    fi
                    return 0
                    ;;
                --base-dir)
                    COMPREPLY=()
                    if [[ "${BASH_VERSINFO[0]}" -ge 4 ]]; then
                        compopt -o plusdirs
                    fi
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        run__subcmd__shell__subcmd__jump__subcmd__remove)
            opts="-h -V --help --version [PATHS]..."
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
    esac
}

if [[ "${BASH_VERSINFO[0]}" -eq 4 && "${BASH_VERSINFO[1]}" -ge 4 || "${BASH_VERSINFO[0]}" -gt 4 ]]; then
    complete -F _shell-jump -o nosort -o bashdefault -o default shell-jump
else
    complete -F _shell-jump -o bashdefault -o default shell-jump
fi
