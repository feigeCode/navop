[[ -n "${_ONETCLI_SHELL_INTEGRATED:-}" ]] && return
export _ONETCLI_SHELL_INTEGRATED=1

__onetcli_emit_osc() {
    printf '\033]%s\007' "$1"
}

__onetcli_prompt_start() {
    __onetcli_emit_osc '133;A'
}

__onetcli_prompt_end() {
    __onetcli_emit_osc '133;B'
}

__onetcli_command_start() {
    __onetcli_emit_osc '133;C'
}

__onetcli_command_done() {
    __onetcli_emit_osc "133;D;$1"
}

__onetcli_update_cwd() {
    __onetcli_emit_osc "7;file://${HOSTNAME:-$(hostname)}$PWD"
}

__onetcli_encode_command() {
    command -v base64 >/dev/null 2>&1 || return 1
    printf '%s' "$1" | base64 | tr -d '\r\n'
}

__onetcli_last_history_command() {
    if [[ -n "${ZSH_VERSION:-}" ]]; then
        fc -ln -1 2>/dev/null | sed 's/^[[:space:]]*//'
    else
        history 1 2>/dev/null | sed 's/^[[:space:]]*[0-9][0-9]*[* ]*[[:space:]]*//'
    fi
}

__onetcli_emit_recorded_command() {
    local command_text encoded
    command_text="$(__onetcli_last_history_command)"
    [[ -z "$command_text" ]] && return
    [[ "$command_text" == "${__ONETCLI_LAST_EMITTED:-}" ]] && return

    encoded="$(__onetcli_encode_command "$command_text")" || return 0
    __ONETCLI_LAST_EMITTED="$command_text"
    __onetcli_emit_osc "1337;Command=${encoded}"
}

__onetcli_precmd_common() {
    local exit_code="$1"
    __onetcli_command_done "$exit_code"
    if [[ -n "${__ONETCLI_COMMAND_STARTED:-}" ]]; then
        __onetcli_emit_recorded_command
        unset __ONETCLI_COMMAND_STARTED
    fi
    __onetcli_update_cwd
    __onetcli_prompt_start
}

if [[ -n "${ZSH_VERSION:-}" ]]; then
    __onetcli_precmd_zsh() {
        __onetcli_precmd_common "$?"
    }

    __onetcli_preexec_zsh() {
        __ONETCLI_COMMAND_STARTED=1
        __onetcli_command_start
    }

    precmd_functions+=(__onetcli_precmd_zsh)
    preexec_functions+=(__onetcli_preexec_zsh)
    PROMPT="${PROMPT}"$'%{\033]133;B\007%}'
else
    __onetcli_precmd_bash() {
        local exit_code="$?"
        __ONETCLI_IN_PRECMD=1
        __onetcli_precmd_common "$exit_code"
        __ONETCLI_IN_PRECMD=0
    }

    __onetcli_preexec_bash() {
        [[ "${__ONETCLI_IN_PRECMD:-0}" == "1" ]] && return
        [[ "${BASH_COMMAND:-}" == __onetcli_* ]] && return
        __ONETCLI_COMMAND_STARTED=1
        __onetcli_command_start
    }

    if [[ -z "${PROMPT_COMMAND:-}" ]]; then
        PROMPT_COMMAND='__onetcli_precmd_bash'
    else
        PROMPT_COMMAND="__onetcli_precmd_bash;${PROMPT_COMMAND}"
    fi

    PS1="${PS1}"$'\033]133;B\007'
    trap '__onetcli_preexec_bash' DEBUG
fi
