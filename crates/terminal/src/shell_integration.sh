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

    # 提示符框架（oh-my-posh / powerlevel10k / starship 等）每个 precmd 都会重写
    # PROMPT，把上面补的结束标记冲掉；导航端就永远等不到 OSC 133;B，握手卡到
    # 30 秒看门狗（表现为「连上后要等 30 秒才能输入」）。这里再挂一个追加到最后
    # 的 precmd 钩子，每轮按需把标记补回 PROMPT 末尾。
    __onetcli_prompt_trailer_zsh() {
        if [[ "${PROMPT:-}" != *$'\033]133;B\007'* ]]; then
            PROMPT="${PROMPT:-}"$'%{\033]133;B\007%}'
        fi
        return 0
    }
    precmd_functions+=(__onetcli_prompt_trailer_zsh)
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

    # 同上（zsh 分支）：oh-my-posh / starship 这类框架每轮重写 PS1，会把上面补的
    # 结束标记冲掉，握手拿不到 OSC 133;B 就卡满看门狗。注册时必须排在框架之后
    # （注入发生在登录 rc 执行完之后，追加到 PROMPT_COMMAND 末尾即可），并在每轮
    # 提示符按需补回标记。
    __onetcli_prompt_trailer_bash() {
        if [[ -n "${PS1:-}" && "${PS1}" != *$'\033]133;B\007'* ]]; then
            PS1="${PS1}"$'\\[\033]133;B\007\\]'
        fi
        return 0
    }
    # 不写 `command -v ... &&` 那种存在性判断：钩子里的每条命令都会触发上面的
    # DEBUG trap（多一个假的 133;C）。函数名本身就以 __onetcli_ 开头，trap 会忽略；
    # 函数缺失时（PROMPT_COMMAND 被导出到子 shell）重定向已把报错吞掉。
    __ONETCLI_PROMPT_TRAILER_HOOK='__onetcli_prompt_trailer_bash 2>/dev/null'

    # bash 5.1+ 的 PROMPT_COMMAND 可能是数组，按实际形态追加到末尾。
    if [[ $(declare -p PROMPT_COMMAND 2>/dev/null) == "declare -a"* ]]; then
        PROMPT_COMMAND+=("$__ONETCLI_PROMPT_TRAILER_HOOK")
    else
        PROMPT_COMMAND="${PROMPT_COMMAND:+${PROMPT_COMMAND}; }$__ONETCLI_PROMPT_TRAILER_HOOK"
    fi

    trap '__onetcli_preexec_bash' DEBUG
fi
