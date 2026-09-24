# 仅在交互式 shell 中生效，避免污染 rsync/scp 等非交互通道
[[ $- != *i* ]] && return
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
        HISTTIMEFORMAT= history 1 2>/dev/null | sed 's/^[[:space:]]*[0-9][0-9]*[* ]*[[:space:]]*//'
    fi
}

__onetcli_emit_recorded_command() {
    local command_text encoded
    command_text="$(__onetcli_last_history_command)"
    [[ -z "$command_text" ]] && return

    encoded="$(__onetcli_encode_command "$command_text")" || return 0
    __onetcli_emit_osc "1337;Command=${encoded}"
}

__onetcli_precmd_common() {
    local exit_code="$1"
    __onetcli_command_done "$exit_code"
    if [[ -n "${_ONETCLI_RUNTIME_SETUP:-}" ]]; then
        unset _ONETCLI_RUNTIME_SETUP __ONETCLI_COMMAND_STARTED
    elif [[ -n "${__ONETCLI_COMMAND_STARTED:-}" ]]; then
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
        # 退出码由 PROMPT_COMMAND 钩子显式传入；直接调用（无参数）时回退到 $?。
        local exit_code="${1:-$?}"
        unset __ONETCLI_EXIT
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

    # PROMPT_COMMAND 的值可能被环境导出，并被之后的子 shell（su、tmux、exec bash 等）
    # 继承，而那些 shell 里并没有本文件的函数定义；钩子若只放裸函数名，它们的每个提示符
    # 都会多输出一行“未找到命令”（issue #217）。因此钩子自带存在性判断：
    # 函数缺失时静默跳过，退出码先取好再显式传给函数。
    __ONETCLI_PROMPT_HOOK='__ONETCLI_EXIT=$?;command -v __onetcli_precmd_bash >/dev/null 2>&1&&__onetcli_precmd_bash "$__ONETCLI_EXIT"'

    if [[ -z "${PROMPT_COMMAND:-}" ]]; then
        PROMPT_COMMAND="$__ONETCLI_PROMPT_HOOK"
    else
        PROMPT_COMMAND="$__ONETCLI_PROMPT_HOOK;${PROMPT_COMMAND}"
    fi

    PS1="${PS1}"$'\\[\033]133;B\007\\]'
    trap '__onetcli_preexec_bash' DEBUG
fi
