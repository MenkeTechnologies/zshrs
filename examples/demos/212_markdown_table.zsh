#!/usr/bin/env zshrs
# Markdown table renderer with column alignment.

render_table() {
    local -a cols=()
    local -a rows=()
    local mode=cols
    while (( $# > 0 )); do
        case $1 in
            --row) mode=row; shift ;;
            --col) mode=col; shift ;;
            *)
                if [[ $mode == row ]]; then
                    rows+=("$1")
                else
                    cols+=("$1")
                fi
                shift
                ;;
        esac
    done

    # Compute column widths.
    local -a widths=()
    for i in {1..${#cols[@]}}; do widths[i]=${#cols[i]}; done
    for row in "${rows[@]}"; do
        local -a fields=( ${(s/|/)row} )
        for ((j=1; j<=${#fields[@]}; j++)); do
            if (( ${#fields[j]} > widths[j] )); then widths[j]=${#fields[j]}; fi
        done
    done

    # Render header.
    printf "|"
    for ((i=1; i<=${#cols[@]}; i++)); do
        printf " %-*s |" ${widths[i]} "${cols[i]}"
    done
    printf "\n|"
    for ((i=1; i<=${#cols[@]}; i++)); do
        local dashes=""
        for ((k=0; k<widths[i]+2; k++)); do dashes+="-"; done
        printf "%s|" "$dashes"
    done
    printf "\n"

    # Render rows.
    for row in "${rows[@]}"; do
        local -a fields=( ${(s/|/)row} )
        printf "|"
        for ((j=1; j<=${#cols[@]}; j++)); do
            printf " %-*s |" ${widths[j]} "${fields[j]:-}"
        done
        printf "\n"
    done
}

echo "── basic users table ──"
render_table \
    --col Name --col Age --col Role \
    --row "Alice|30|Admin" \
    --row "Bob|25|User" \
    --row "Carol|35|Guest" \
    --row "Dave|40|Owner"

echo
echo "── compact data table ──"
render_table \
    --col ID --col Status \
    --row "1|active" \
    --row "2|pending" \
    --row "3|active"

echo
echo "── variable widths ──"
render_table \
    --col Item --col Description --col Qty \
    --row "Apple|crisp red fruit|10" \
    --row "Banana|yellow tropical|5" \
    --row "Elderberry|small purple|200"
