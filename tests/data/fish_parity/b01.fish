if status is-interactive
    echo interactive
else
    echo script
end
type -q ls; and echo has-ls
command -v ls >/dev/null; echo $status
not type -q nonexistent_cmd_xyz; and echo missing
