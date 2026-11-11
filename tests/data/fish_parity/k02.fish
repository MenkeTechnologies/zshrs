echo stderr-only 2>| string upper
begin; echo out; echo err >&2; end 2>| string upper
echo a | string upper
    # comment between
    | string repeat -n 2
