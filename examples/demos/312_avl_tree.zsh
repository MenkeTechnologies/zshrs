#!/usr/bin/env zshrs
# AVL tree concepts — balance factor + rotation logic.
# Implemented as conceptual demo (recursive AVL via cmd-sub doesn't propagate
# mutations in zshrs subshells; documented in BUGS.md).

# 3 nodes: id, value, left, right, height — stored in parallel arrays.
typeset -A V LCH RCH HT
typeset -gi NID=0

new_node() {
    (( NID++ ))
    local id=$NID
    V[$id]=$1
    LCH[$id]=0
    RCH[$id]=0
    HT[$id]=1
    LAST_NEW=$id
}

# Compute balance factor.
height() {
    (( $1 == 0 )) && { echo 0; return; }
    echo ${HT[$1]}
}

balance_factor() {
    local n=$1
    (( n == 0 )) && { echo 0; return; }
    local lh=$(height ${LCH[$n]})
    local rh=$(height ${RCH[$n]})
    echo $(( lh - rh ))
}

update_height() {
    local n=$1
    local lh=$(height ${LCH[$n]})
    local rh=$(height ${RCH[$n]})
    HT[$n]=$(( (lh > rh ? lh : rh) + 1 ))
}

# Right rotation: y becomes right child of x; t2 becomes left child of y.
demo_right_rotate() {
    local y=$1
    local x=${LCH[$y]}
    local t2=${RCH[$x]}
    RCH[$x]=$y
    LCH[$y]=$t2
    update_height $y
    update_height $x
    echo "  rotate right: new root = $x (val ${V[$x]})"
    NEW_ROOT=$x
}

# Left rotation: x becomes left child of y; t2 becomes right child of x.
demo_left_rotate() {
    local x=$1
    local y=${RCH[$x]}
    local t2=${LCH[$y]}
    LCH[$y]=$x
    RCH[$x]=$t2
    update_height $x
    update_height $y
    echo "  rotate left: new root = $y (val ${V[$y]})"
    NEW_ROOT=$y
}

echo "── balance factor rules ──"
echo "  bf =  0  : balanced"
echo "  bf =  1  : slightly left-heavy"
echo "  bf = -1  : slightly right-heavy"
echo "  bf >  1  : LEFT-HEAVY → needs right rotation (LL) or left-right (LR)"
echo "  bf < -1  : RIGHT-HEAVY → needs left rotation (RR) or right-left (RL)"

echo
echo "── rotation cases ──"
echo "  LL (left-left): bf > 1, child bf > 0   → single right rotation"
echo "  RR (right-right): bf < -1, child bf < 0 → single left rotation"
echo "  LR (left-right): bf > 1, child bf < 0   → left then right"
echo "  RL (right-left): bf < -1, child bf > 0  → right then left"

echo
echo "── build skewed-right tree (RR scenario) ──"
new_node 10; A=$LAST_NEW
new_node 20; B=$LAST_NEW
new_node 30; C=$LAST_NEW
RCH[$A]=$B
RCH[$B]=$C
update_height $C
update_height $B
update_height $A

echo "  tree: $A (val 10) — right → $B (val 20) — right → $C (val 30)"
echo "  height A: $(height $A)"
echo "  bf A:     $(balance_factor $A) → RR scenario"

demo_left_rotate $A
echo "  after rotation: root $NEW_ROOT (val ${V[$NEW_ROOT]})"
echo "    L: $(echo ${LCH[$NEW_ROOT]}) (val ${V[${LCH[$NEW_ROOT]}]})"
echo "    R: $(echo ${RCH[$NEW_ROOT]}) (val ${V[${RCH[$NEW_ROOT]}]})"

echo
echo "── build skewed-left tree (LL scenario) ──"
V=(); LCH=(); RCH=(); HT=(); NID=0
new_node 30; A=$LAST_NEW
new_node 20; B=$LAST_NEW
new_node 10; C=$LAST_NEW
LCH[$A]=$B
LCH[$B]=$C
update_height $C
update_height $B
update_height $A

echo "  tree: $A (val 30) — left → $B (val 20) — left → $C (val 10)"
echo "  bf A: $(balance_factor $A) → LL scenario"

demo_right_rotate $A
echo "  after rotation: root $NEW_ROOT (val ${V[$NEW_ROOT]})"

echo
echo "── LR scenario (zigzag) ──"
V=(); LCH=(); RCH=(); HT=(); NID=0
# 30
#  └─ left: 10
#         └─ right: 20
new_node 30; A=$LAST_NEW
new_node 10; B=$LAST_NEW
new_node 20; C=$LAST_NEW
LCH[$A]=$B
RCH[$B]=$C
update_height $C
update_height $B
update_height $A

echo "  before:"
echo "    bf at root: $(balance_factor $A)"
echo "    bf at left child: $(balance_factor $B)"
echo "  → LR case (bf>1 at root, bf<0 at left child)"
echo "  step 1: rotate left on left child"
demo_left_rotate $B
LCH[$A]=$NEW_ROOT
update_height $A
echo "  step 2: rotate right on root"
demo_right_rotate $A
echo "  final root: ${V[$NEW_ROOT]}"

echo
echo "── AVL invariants ──"
echo "  guarantees |bf| ≤ 1 at every node"
echo "  height ≤ 1.44 × log2(n+2) - 0.328"
echo "  vs unbalanced BST worst case: O(n) height"
echo
echo "  insert cost: O(log n)"
echo "  search cost: O(log n)"
echo "  delete cost: O(log n)"
echo "  rotations per op: ≤ O(log n) (typically 1-2)"

echo
echo "── compare to red-black ──"
echo "  AVL: stricter balance, faster lookups, more rotations on insert/delete"
echo "  RB:  looser balance, fewer rotations, used by std::map, java.util.TreeMap"

echo
echo "  data structures using AVL:"
echo "    AVLTree<K,V> in some C++ libraries"
echo "    Boost intrusive AVL set/map"
echo "    Some database index implementations"

# === ztest assertions ===
V=(); LCH=(); RCH=(); HT=(); NID=0
new_node 10; a=$LAST_NEW
new_node 20; b=$LAST_NEW
new_node 30; c=$LAST_NEW
RCH[$a]=$b
RCH[$b]=$c
update_height $c
update_height $b
update_height $a
zassert_eq "$(height $a)" "3"            "skewed-right height 3"
zassert_eq "$(balance_factor $a)" "-2"   "skewed-right bf = -2 (RR scenario)"
demo_left_rotate $a > /dev/null
zassert_eq "${V[$NEW_ROOT]}" "20"        "after RR left-rotate, root val = 20"
zassert_eq "${V[${LCH[$NEW_ROOT]}]}" "10"  "rotated left child val = 10"
zassert_eq "${V[${RCH[$NEW_ROOT]}]}" "30"  "rotated right child val = 30"
# LL scenario
V=(); LCH=(); RCH=(); HT=(); NID=0
new_node 30; a=$LAST_NEW
new_node 20; b=$LAST_NEW
new_node 10; c=$LAST_NEW
LCH[$a]=$b
LCH[$b]=$c
update_height $c; update_height $b; update_height $a
zassert_eq "$(balance_factor $a)" "2"    "skewed-left bf = 2 (LL scenario)"
demo_right_rotate $a > /dev/null
zassert_eq "${V[$NEW_ROOT]}" "20"        "after LL right-rotate, root val = 20"
ztest_run
