#!/usr/bin/env zshrs
# Caesar cipher — single-shift substitution + ROT13.

shift_char() {
    local c=$1 n=$2
    case $c in
        [A-Z])
            local i=$(( (#c - 65 + n + 26) % 26 ))
            printf "\\$(printf %03o $((i + 65)))"
            ;;
        [a-z])
            local i=$(( (#c - 97 + n + 26) % 26 ))
            printf "\\$(printf %03o $((i + 97)))"
            ;;
        *) printf "%s" "$c" ;;
    esac
}

caesar() {
    local s=$1 n=$2 out=""
    local i
    for ((i=1; i<=${#s}; i++)); do
        out+="$(shift_char "${s[i]}" "$n")"
    done
    echo "$out"
}

rot13() { caesar "$1" 13; }

echo "── shift 3 (classical Caesar) ──"
text="THE QUICK BROWN FOX JUMPS OVER THE LAZY DOG"
enc=$(caesar "$text" 3)
echo "  plain:  $text"
echo "  enc(3): $enc"
echo "  dec(3): $(caesar "$enc" -3)"

echo
echo "── ROT13 (self-inverse) ──"
text="Hello, World!"
e=$(rot13 "$text")
ee=$(rot13 "$e")
echo "  plain:    $text"
echo "  rot13:    $e"
echo "  rot13^2:  $ee   $([[ $ee == $text ]] && echo ✓ || echo ✗)"

echo
echo "── all shifts (1..25) of 'attack' ──"
for n in {1..25}; do
    printf "  shift %2d: %s\n" $n "$(caesar "attack" $n)"
done

echo
echo "── brute-force a Caesar ciphertext ──"
secret="Khoor#Zruog"  # "Hello World" shifted by 3
echo "  ciphertext: $secret"
echo "  candidates:"
for n in {1..25}; do
    cand=$(caesar "$secret" -$n)
    printf "    -%2d → %s\n" $n "$cand"
done

echo
echo "── mixed case preserved ──"
text="MiXeD Case123"
echo "  $text → $(caesar "$text" 5)"

echo
echo "── punctuation passthrough ──"
text="A.B,C!D?E"
echo "  $text → $(caesar "$text" 1)"

# === ztest assertions ===
zassert_eq "$(shift_char A 1)" "B" "A+1=B"
zassert_eq "$(shift_char Z 1)" "A" "Z+1=A (wrap)"
zassert_eq "$(shift_char a 1)" "b" "a+1=b"
zassert_eq "$(shift_char z 1)" "a" "z+1=a (wrap)"
zassert_eq "$(shift_char A -1)" "Z" "A-1=Z (back-wrap)"
zassert_eq "$(shift_char ',' 5)" "," "comma passthrough"
zassert_eq "$(caesar HELLO 1)" "IFMMP" "HELLO+1"
zassert_eq "$(caesar IFMMP -1)" "HELLO" "IFMMP-1"
zassert_eq "$(caesar 'THE QUICK BROWN FOX' 3)" "WKH TXLFN EURZQ IRA" "spaces preserved"
zassert_eq "$(rot13 hello)"     "uryyb" "rot13 hello"
zassert_eq "$(rot13 'Hello, World!')" "Uryyb, Jbeyq!" "rot13 mixed"
zassert_eq "$(rot13 $(rot13 abcXYZ))" "abcXYZ" "rot13 self-inverse"
zassert_eq "$(caesar Khoor#Zruog -3)" "Hello#World" "brute-force shift -3"
zassert_eq "$(caesar attack 0)" "attack" "shift 0 = identity"
zassert_eq "$(caesar attack 26)" "attack" "shift 26 = identity (wraps)"
ztest_run
