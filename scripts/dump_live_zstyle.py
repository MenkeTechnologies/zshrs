#!/usr/bin/env python3
"""dump_live_zstyle.py — regenerate the parity harness's zstyle fixture from
the live interactive shell.

    scripts/dump_live_zstyle.py scripts/parity_zstyle.zsh

Why a PTY and not `zsh -i -c 'zstyle -L'`: zinit turbo defers most plugin
loading to `precmd`, so a non-interactive `-c` run reports only the styles the
rc sets synchronously (21 of 207 on the author's host). Reaching several real
prompts on a pty lets the deferred loads finish, which is the config the shell
is actually completing under.

Two rewrites are applied to the captured statements, both preserving the style
and the value CARDINALITY so list/menu rendering is exercised identically:

  * absolute home paths -> ``${HOME}``
  * ``hosts`` value lists -> neutral placeholders. Real hostnames and addresses
    are infrastructure detail and never belong in the repo.

`zstyle -L` emits re-sourceable shell, so the output needs no extra quoting.
"""

from __future__ import annotations

import argparse
import os
import pty
import re
import select
import sys
import time

# Same count as a realistic host list: enough entries to span several list
# columns and force paging at 24x80, which is the point of keeping cardinality.
PLACEHOLDER_HOSTS = " ".join(
    [f"h{i}" for i in range(1, 21)]
    + [f"host{i:02d}.example.com" for i in range(1, 11)]
    + [f"192.0.2.{i}" for i in range(1, 9)]
)

HEADER = """\
# zstyle fixture for the compsys parity harnesses — the real daily-driver
# completion config, captured from a live interactive session.
#
# Sourced identically into `zsh -f` and into zshrs (both `--zsh` emulation and
# the native `-f -i` path) so the harness compares the completion ENGINE under
# the actual config, not under defaults.
#
# Regenerate with:
#     scripts/dump_live_zstyle.py scripts/parity_zstyle.zsh
#
# `zstyle -L` output is re-sourceable as-is; the generator only rewrites two
# things, both of which keep the STYLE and the CARDINALITY intact so list/menu
# rendering is exercised the same way:
#   * absolute home paths  -> ${HOME}
#   * `hosts` value lists  -> neutral placeholders (never commit real hostnames
#                             or addresses to the repo)
#
# Capturing this needs a PTY that reaches several prompts: zinit turbo defers
# most plugin loading to precmd, so `zsh -i -c 'zstyle -L'` reports only a
# fraction of the styles (21 of 207 on this host).
"""


def capture(shell: str, settle: float, prompts: int, tmp: str) -> str:
    """Run `shell -i` on a pty, let it settle, dump `zstyle -L` to `tmp`."""
    pid, fd = pty.fork()
    if pid == 0:  # child
        os.execvpe(shell, [shell, "-i"], os.environ)

    def drain(seconds: float) -> None:
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            r, _, _ = select.select([fd], [], [], 0.2)
            if not r:
                continue
            try:
                if not os.read(fd, 65536):
                    return
            except OSError:
                return

    drain(settle)
    # Each command execution fires precmd, which is what turbo hooks off.
    for _ in range(prompts):
        os.write(fd, b"true\n")
        drain(4.0)
    os.write(fd, f"zstyle -L > {tmp}\n".encode())
    drain(6.0)
    os.write(fd, b"exit\n")
    drain(3.0)
    for closer in (lambda: os.close(fd), lambda: os.waitpid(pid, os.WNOHANG)):
        try:
            closer()
        except OSError:
            pass
    if not os.path.exists(tmp):
        sys.exit(
            "dump_live_zstyle: shell never wrote the dump — raise --settle "
            "(the rc may still have been loading)"
        )
    return tmp


def sanitize(raw_path: str) -> list[str]:
    home = os.path.expanduser("~")
    out = []
    for line in open(raw_path):
        line = line.rstrip("\n")
        if not line.startswith("zstyle "):
            continue
        line = line.replace(home, "${HOME}")
        m = re.match(r"^(zstyle \S+ hosts )", line)
        if m:
            line = m.group(1) + PLACEHOLDER_HOSTS
        # zsh-syntax-highlighting records the widgets it wrapped under names
        # carrying the capturing session's tty id (`orig-s000-r148-...`). That
        # id changes on every capture, so leaving it in makes the fixture — and
        # therefore every generated combo — churn for no behavioural reason.
        line = re.sub(r"orig-s\d+-r\d+-", "orig-s000-r000-", line)
        out.append(line)
    return sorted(set(out))


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("out", help="fixture path to write")
    ap.add_argument("--from-file", default=None, metavar="PATH",
                    help="sanitize an EXISTING `zstyle -L` dump instead of "
                         "capturing one (e.g. scripts/zstyle.zsh, which is "
                         "gitignored because it holds real hosts and paths)")
    ap.add_argument("--shell", default="/bin/zsh", help="interactive shell to capture")
    ap.add_argument("--settle", type=float, default=60.0,
                    help="seconds to let the rc + turbo plugins load")
    ap.add_argument("--prompts", type=int, default=4,
                    help="how many precmd cycles to drive after settling")
    args = ap.parse_args()

    if args.from_file:
        if not os.path.exists(args.from_file):
            sys.exit(f"dump_live_zstyle: no such dump: {args.from_file}")
        statements = sanitize(args.from_file)
    else:
        tmp = args.out + ".raw"
        capture(args.shell, args.settle, args.prompts, tmp)
        statements = sanitize(tmp)
        os.unlink(tmp)
    if len(statements) < 50:
        sys.exit(
            f"dump_live_zstyle: only {len(statements)} statements captured — "
            "that is the pre-turbo subset, raise --settle"
        )
    with open(args.out, "w") as f:
        f.write(HEADER)
        f.write("\n".join(statements) + "\n")
        # Several captured styles are function-VALUED and name functions that
        # ship with zpwr/fasd, not with zsh (cache-policy zpwrMonthlyCachingPolicy,
        # completer _megacomplete / _fasd_zsh_word_complete*). Undefined, compsys
        # silently takes a different path — an unknown completer is skipped and a
        # missing cache-policy means "always rebuild" — so the fixture would model
        # a different completer chain than the one captured. The companion file
        # defines them; emit the source line here so a regeneration cannot drop it.
        # INLINED, not sourced: the fixture must be a single self-contained
        # file. A sourced sibling needs a path expression and every such
        # expression is a footgun — `${0:A:h}` silently resolves against $PWD
        # when FUNCTION_ARGZERO is unset, so the definitions would quietly not
        # load and the fixture would model a different completer chain.
        stubs = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                             "parity_zstyle_stubs.zsh")
        text = open(stubs).read()
        f.write("\n" + text[text.index("# --- cache-policy"):])
    print(f"{args.out}: {len(statements)} statements")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
