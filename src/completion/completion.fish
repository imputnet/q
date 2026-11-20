function __q_complete
    set -l tokens (commandline -opc)
    set -l cur (commandline -ct)
    q __complete $tokens -- $cur
end
complete -c q -a '(__q_complete)'
complete -c quilt -a '(__q_complete)'
