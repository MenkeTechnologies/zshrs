function a; echo A; end
function b; a; echo B; end
b
functions -q a; and echo a-exists
functions -e a
functions -q a; or echo a-gone
