# TODO — porting gaps that block 100% C-faithful ports

This file tracks port gaps where a function in `src/ported/` cannot
yet be faithfully ported because it depends on a C primitive that
hasn't been ported. Each entry blocks at least one downstream file
from being marked as 100% line-by-line in `docs/PORT_CHECKLIST.md`.

When an item is fixed: port the dependency, fix every blocked
function, then strike through (or delete) the entry here.

---

## Note: this file is the only place to mention port gaps

If a function in `src/ported/` is not 100% line-by-line, it
must NOT be ticked in `docs/PORT_CHECKLIST.md`. Add the gap
here, link from the file's checklist entry, and keep moving.
When the gap is closed, delete the entry.
