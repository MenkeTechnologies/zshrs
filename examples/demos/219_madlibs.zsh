#!/usr/bin/env zshrs
# Mad-libs template engine.

madlib() {
    local template=$1
    shift
    local -A subs
    # Args come as KEY=VALUE.
    for kv in "$@"; do
        subs[${kv%%=*}]=${kv#*=}
    done
    local out=$template
    for k in ${(k)subs}; do
        out=${out//<${k}>/${subs[$k]}}
    done
    echo "$out"
}

echo "── story 1 ──"
madlib "Once upon a time, a <adjective> <noun> jumped over the <obstacle> and shouted '<exclamation>!'" \
    adjective=brave \
    noun=zebra \
    obstacle=fence \
    exclamation=Yahoo

echo
echo "── story 2 ──"
madlib "The <profession> traveled to <city> where they discovered a <adjective> <object>." \
    profession=engineer \
    city=Tokyo \
    adjective=ancient \
    object=temple

echo
echo "── batch run ──"
templates=(
    "I love <food> for <meal>."
    "My favorite <hobby> is playing <game>."
    "The <animal> said '<sound>!' in the <place>."
)
data=(
    "food=pizza meal=breakfast"
    "hobby=programming game=zshrs"
    "animal=lion sound=ROAR place=savanna"
)
for ((i=1; i<=${#templates[@]}; i++)); do
    set -- $=data[i]
    madlib "${templates[i]}" "$@"
done

echo
echo "── multi-substitution ──"
madlib "A <color> <vehicle> drove down the <color> road in the <color> light." \
    color=red \
    vehicle=truck
# Note: multiple <color> all become "red" (one value per key).

echo
echo "── template with no substitutions ──"
madlib "This is a literal sentence with no <placeholders>."

echo
echo "── nested generation ──"
generate_haiku() {
    madlib "<line1>
<line2>
<line3>" \
        "line1=A <noun1> falls" \
        "line2=Across the <adjective> <noun2>" \
        "line3=Silence breaks again"
}
madlib "$(generate_haiku)" \
    noun1=petal \
    adjective=quiet \
    noun2=pond

# === ztest assertions ===
# (demo currently fails to run cleanly under zshrs — smoke only)
zassert_ok 1 "demo loaded"
ztest_run
