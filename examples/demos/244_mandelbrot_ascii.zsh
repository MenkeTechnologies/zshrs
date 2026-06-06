#!/usr/bin/env zshrs
# Mandelbrot set — ASCII rendering, integer math for portability.
# Uses scaled fixed-point arithmetic since zsh has shaky float support.

zmodload zsh/mathfunc 2>/dev/null

# Scale factor (1024 = 10-bit fixed point).
S=1024

WIDTH=40
HEIGHT=16
MAX_ITER=20

# Map screen (x,y) → complex plane (-2.5..1.0, -1.0..1.0).
# Use integer math: cx in [-2560..1024] / 1024.
echo "── Mandelbrot (60x24, max 30 iter) ──"
for ((py=0; py<HEIGHT; py++)); do
    line=""
    for ((px=0; px<WIDTH; px++)); do
        # cx = -2.5 + 3.5 * px / WIDTH; cy = -1.0 + 2.0 * py / HEIGHT
        cx=$(( -2500 + 3500 * px / WIDTH ))
        cy=$(( -1000 + 2000 * py / HEIGHT ))
        zx=0
        zy=0
        iter=0
        while (( iter < MAX_ITER )); do
            # |z|^2 = zx^2 + zy^2; if > 4 (scaled: > 4 * 1000 * 1000 = 4_000_000), escape.
            zx2=$(( zx * zx / 1000 ))
            zy2=$(( zy * zy / 1000 ))
            if (( zx2 + zy2 > 4000 )); then break; fi
            # z = z^2 + c; zx' = zx2 - zy2 + cx; zy' = 2*zx*zy/1000 + cy
            new_zx=$(( zx2 - zy2 + cx ))
            new_zy=$(( 2 * zx * zy / 1000 + cy ))
            zx=$new_zx
            zy=$new_zy
            (( iter++ ))
        done
        if (( iter == MAX_ITER )); then
            line+="█"
        elif (( iter > 15 )); then
            line+="▓"
        elif (( iter > 7 )); then
            line+="▒"
        elif (( iter > 3 )); then
            line+="░"
        else
            line+=" "
        fi
    done
    echo "$line"
done

echo
echo "── escape-time stats ──"
total=$((WIDTH * HEIGHT))
echo "  grid: ${WIDTH}x${HEIGHT} = $total pixels"
echo "  max iter: $MAX_ITER"
echo "  bailout: |z|^2 > 4"
echo "  region: x∈[-2.5,1.0], y∈[-1.0,1.0]"

echo
echo "── verify a known interior point: (-0.5, 0) ──"
# c = (-500/1000, 0/1000) in scaled units.
cx=-500; cy=0
zx=0; zy=0; iter=0
while (( iter < 100 )); do
    zx2=$(( zx * zx / 1000 ))
    zy2=$(( zy * zy / 1000 ))
    if (( zx2 + zy2 > 4000 )); then break; fi
    new_zx=$(( zx2 - zy2 + cx ))
    new_zy=$(( 2 * zx * zy / 1000 + cy ))
    zx=$new_zx
    zy=$new_zy
    (( iter++ ))
done
echo "  (-0.5, 0): iter=$iter (in-set if 100)"

echo
echo "── known exterior point: (1, 1) ──"
cx=1000; cy=1000
zx=0; zy=0; iter=0
while (( iter < 100 )); do
    zx2=$(( zx * zx / 1000 ))
    zy2=$(( zy * zy / 1000 ))
    if (( zx2 + zy2 > 4000 )); then break; fi
    new_zx=$(( zx2 - zy2 + cx ))
    new_zy=$(( 2 * zx * zy / 1000 + cy ))
    zx=$new_zx
    zy=$new_zy
    (( iter++ ))
done
echo "  (1, 1): iter=$iter (escaped early)"

# === ztest assertions ===
zassert_eq "$WIDTH"    "40" "WIDTH set"
zassert_eq "$HEIGHT"   "16" "HEIGHT set"
zassert_eq "$MAX_ITER" "20" "MAX_ITER set"
zassert_eq "$S"        "1024" "fixed-point scale"
# Recompute c=(-0.5,0) — should iterate to 100.
cx=-500; cy=0; zx=0; zy=0; iter=0
while (( iter < 100 )); do
    zx2=$(( zx * zx / 1000 ))
    zy2=$(( zy * zy / 1000 ))
    if (( zx2 + zy2 > 4000 )); then break; fi
    new_zx=$(( zx2 - zy2 + cx ))
    new_zy=$(( 2 * zx * zy / 1000 + cy ))
    zx=$new_zx; zy=$new_zy
    (( iter++ ))
done
zassert_eq "$iter" "100" "(-0.5, 0) is in the Mandelbrot set"
# c=(1,1) — should escape quickly.
cx=1000; cy=1000; zx=0; zy=0; iter=0
while (( iter < 100 )); do
    zx2=$(( zx * zx / 1000 ))
    zy2=$(( zy * zy / 1000 ))
    if (( zx2 + zy2 > 4000 )); then break; fi
    new_zx=$(( zx2 - zy2 + cx ))
    new_zy=$(( 2 * zx * zy / 1000 + cy ))
    zx=$new_zx; zy=$new_zy
    (( iter++ ))
done
zassert_lt "$iter" "10"  "(1,1) escapes within 10 iterations"
zassert_eq "$((WIDTH * HEIGHT))" "640" "total pixel count"
ztest_run
