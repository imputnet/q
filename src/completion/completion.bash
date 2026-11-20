_q_complete() {
    local cur
    cur="${COMP_WORDS[COMP_CWORD]}"
    COMPREPLY=( $(COMP_CWORD="$COMP_CWORD" q __complete "${COMP_WORDS[@]:1}" -- "$cur") )
    local reply
    for reply in "${COMPREPLY[@]}"; do
        if [[ "$reply" == */ ]]; then
            compopt -o nospace 2>/dev/null || true
            break
        fi
    done
}
complete -o default -F _q_complete q quilt
