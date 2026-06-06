#!/usr/bin/env zshrs
# Menu-driven app with scripted selections (no interactive read).

typeset -A MENU
typeset -A SUB_MENU

# Main menu options.
MENU[1]="File operations"
MENU[2]="Search"
MENU[3]="Settings"
MENU[4]="Help"
MENU[5]="Quit"

# Submenu for File operations.
SUB_MENU[1.1]="New file"
SUB_MENU[1.2]="Open file"
SUB_MENU[1.3]="Save file"
SUB_MENU[1.4]="Delete file"

# Submenu for Settings.
SUB_MENU[3.1]="Theme"
SUB_MENU[3.2]="Keybindings"
SUB_MENU[3.3]="Language"

show_menu() {
    local prefix=$1
    local title=$2
    echo "── $title ──"
    if [[ -z $prefix ]]; then
        for k in ${(ko)MENU}; do
            echo "  $k) ${MENU[$k]}"
        done
    else
        for k in ${(ko)SUB_MENU}; do
            if [[ $k == ${prefix}.* ]]; then
                echo "  ${k#${prefix}.}) ${SUB_MENU[$k]}"
            fi
        done
        echo "  0) back"
    fi
}

handle_selection() {
    local prefix=$1 sel=$2
    local key="${prefix:+${prefix}.}${sel}"
    if [[ -n ${MENU[$key]+x} ]]; then
        echo "  selected: ${MENU[$key]}"
    elif [[ -n ${SUB_MENU[$key]+x} ]]; then
        echo "  selected: ${SUB_MENU[$key]}"
    else
        echo "  invalid: $key"
    fi
}

echo "── Main menu (scripted: pick option 1 to enter File ops) ──"
show_menu "" "Main Menu"
handle_selection "" 1

echo
echo "── File ops menu (scripted: pick 2 to Open) ──"
show_menu "1" "File operations"
handle_selection "1" 2

echo
echo "── Settings menu (pick 1 for Theme) ──"
show_menu "3" "Settings"
handle_selection "3" 1

echo
echo "── Invalid selection ──"
handle_selection "" 99

echo
echo "── full menu walk ──"
walk_menu() {
    show_menu "" "Top"
    for k in ${(ko)MENU}; do
        echo ""
        echo "→ selecting $k:"
        echo "  ${MENU[$k]}"
        # Show submenu if exists.
        local has_sub=0
        for sk in ${(k)SUB_MENU}; do
            if [[ $sk == ${k}.* ]]; then
                has_sub=1; break
            fi
        done
        if (( has_sub )); then
            show_menu "$k" "  (sub)"
        fi
    done
}
walk_menu

# === ztest assertions ===
zassert_eq "${MENU[1]}"          "File operations" "MENU[1]"
zassert_eq "${MENU[5]}"          "Quit"             "MENU[5]"
zassert_eq "${SUB_MENU[1.1]}"    "New file"        "submenu 1.1"
zassert_eq "${SUB_MENU[3.2]}"    "Keybindings"     "submenu 3.2"
zassert_eq "${#MENU[@]}"         5                  "5 top-level entries"
zassert_eq "${#SUB_MENU[@]}"     7                  "7 sub entries"
zassert_contains "$(handle_selection '' 1)"   "File operations" "select 1 → File operations"
zassert_contains "$(handle_selection '' 99)"  "invalid"         "invalid selection 99"
zassert_contains "$(handle_selection 3 1)"    "Theme"           "select 3.1 → Theme"
ztest_run
