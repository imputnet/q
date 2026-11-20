_q_complete() {
    local -a completions
    local completion
    local no_space
    completions=()
    no_space=0
    while IFS= read -r completion; do
        completions+=("$completion")
        if [[ "$completion" == */ ]]; then
            no_space=1
        fi
    done < <(q __complete "${words[@]}" -- "$words[CURRENT]")
    if (( ${#completions[@]} )); then
        if (( no_space )); then
            compadd -S '' -- "${completions[@]}"
        else
            compadd -- "${completions[@]}"
        fi
    else
        _files
    fi
}
if ! whence -w compdef >/dev/null; then
    autoload -Uz compinit
    compinit
fi
compdef _q_complete q quilt
