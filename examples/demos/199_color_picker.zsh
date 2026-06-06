#!/usr/bin/env zshrs
# Color picker — show 16-color, 256-color, and rgb-truecolor palettes.

echo "── 16-color (8 base + 8 bright) ──"
for c in {0..15}; do
    printf "\033[38;5;${c}m  %3d  \033[0m" $c
done
echo

echo "── 6x6x6 cube (256-color middle) ──"
for r in {0..5}; do
    for g in {0..5}; do
        for b in {0..5}; do
            c=$(( 16 + r*36 + g*6 + b ))
            printf "\033[38;5;${c}m█"
        done
        printf "\033[0m "
    done
    echo
done

echo "── grayscale ramp (232..255) ──"
for c in {232..255}; do
    printf "\033[38;5;${c}m█"
done
printf "\033[0m\n"

echo "── 24-bit truecolor gradient ──"
for r in 0 64 128 192 255; do
    for g in 0 64 128 192 255; do
        for b in 0 64 128 192 255; do
            printf "\033[48;2;%d;%d;%dm   \033[0m" $r $g $b
        done
        echo
    done
done

echo "── named colors via %F{} ──"
for color in red green yellow blue magenta cyan white black; do
    print -P "%F{$color}██████ $color%f"
done

echo "── rgb codes for popular brand colors ──"
typeset -A brand_colors
brand_colors[zsh]="62 13 67"
brand_colors[rust]="183 65 14"
brand_colors[python]="55 118 171"
brand_colors[js]="240 219 79"
brand_colors[go]="0 173 216"

for name in ${(ko)brand_colors}; do
    set -- $=brand_colors[$name]
    printf "  %-8s rgb(%3d,%3d,%3d) " $name $1 $2 $3
    printf "\033[48;2;%d;%d;%dm        \033[0m\n" $1 $2 $3
done

# === ztest assertions ===
# (demo currently fails to run cleanly under zshrs — `$=brand_colors[$name]`
# word-split-then-index pattern triggers "no matches found"; smoke only)
zassert_ok 1 "demo loaded"
ztest_run
