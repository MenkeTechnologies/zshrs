# Function definitions via the `name() { body }` syntax — caught by
# BUILTIN_REGISTER_COMPILED_FN. (The `function NAME { body }` keyword
# variant goes through a different code path and isn't covered here;
# tracked separately as Phase 2.5.)
hello_one() { echo one; }
hello_two() {
    echo two
    echo more
}
hello_three() { local x=3; echo $x; }
