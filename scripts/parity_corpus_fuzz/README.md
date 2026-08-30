# parity_corpus_fuzz — the completion fuzzer's persistent corpus

Inputs for `scripts/comptab_parity.py --mutate N`, and the place both fuzz
modes (`--mutate` and `--style-fuzz`) write what they find. One small JSON file
per input; an input is a complete cell:

```json
{
  "buffer": "CC ",
  "keys": ["tab"],
  "statements": ["zstyle ':completion:*' completer _expand _ignored ..."],
  "origin": "promoted/mut003",
  "fingerprint": "19376c78e1",
  "note": "cell/row0/col0 | ref/$ CC _main_complete:#: command not found: ..."
}
```

`buffer` is typed, then `keys` are sent one at a time, with `statements`
sourced as the zstyle config. `fingerprint` is set only on a **promoted**
entry — one the fuzzer produced itself.

## Why the directory exists

A one-shot random sweep re-rolls the same dice every run. It cannot get better
at finding bugs, and everything it found dies with the process. This directory
is the memory: when a run hits a failure fingerprint that is not already in
here, the reproducer is minimised in all three dimensions (zstyle statements,
key sequence, buffer) and written back as `fp_<hash>.json`, so the next run
starts from it and mutates around known-bad territory instead of around the
seeds.

Mutation parents are drawn with weights, not uniformly — evidence earns weight:

| entry                                | weight | what it is                              |
| ------------------------------------ | -----: | --------------------------------------- |
| carries a `fingerprint`              |     12 | a reproducer a fuzz run mined and shrank — `origin` is `promoted/*` from `--mutate` or `style-fuzz/*` from `--style-fuzz` |
| `origin` starts `divergent-cases/`   |      6 | a buffer from `comptab_divergent_cases.txt`, each serially confirmed |
| `origin` starts `CASES/`             |      1 | a hand-written case from `parity_corpus.py` |

## What is tracked in git, and what is not

* **Tracked: `fp_*.json` (+ its `*_styles.zsh` sidecar).** These are the
  accumulated findings. They cannot be regenerated from anything — they are
  what the fuzzer learned.
* **Not tracked: `seed_*.json`.** 500+ files that are a mechanical product of
  `parity_corpus.CASES` x `--seed-sequences` plus
  `scripts/comptab_divergent_cases.txt`. Regenerate them in one command; there
  is nothing in them git does not already hold.

```sh
scripts/comptab_parity.py --corpus-seed                       # no zstyles
scripts/comptab_parity.py --corpus-seed --zstyle scripts/parity_zstyle.zsh
```

## Running the fuzzer

Two fuzzers write here. `--mutate` starts from what the corpus already holds
and steps just off it. `--style-fuzz` GENERATES zstyle statements instead of
sampling a fixture, which is the only way the VALUE grammar of a style
(match specifications, completer chain order, tag-order, menu, format,
list-colors, ...) gets exercised at all — subset sampling can only ever replay
values that were already in `scripts/parity_zstyle.zsh`.

```sh
# 20 mutated inputs, anywhere in the corpus
scripts/comptab_parity.py --mutate 20

# fuzz around the mined divergences only
scripts/comptab_parity.py --mutate 20 --corpus-origin divergent-cases

# fuzz around what the fuzzer already found
scripts/comptab_parity.py --mutate 20 --corpus-origin promoted

# 20 GENERATED zstyle configurations, 5 statements each
scripts/comptab_parity.py --style-fuzz 20 --style-fuzz-styles 5

# hammer one surface — every statement is a generated matcher-list
scripts/comptab_parity.py --style-fuzz 40 --style-fuzz-only matcher-list

# compose: a generated config layered on a subset of the real fixture
scripts/comptab_parity.py --style-fuzz 20 --style-fuzz-mix 0.3 \
                          --zstyle scripts/parity_zstyle.zsh

# see what the generator emits, and what zsh makes of it — no shells booted
scripts/comptab_parity.py --style-fuzz-list 40

# replay one promoted reproducer exactly
scripts/comptab_parity.py --zstyle scripts/parity_corpus_fuzz/fp_<hash>_styles.zsh \
                          --case '<buffer>' --keys <keys>
```

Each cell boots two real shells (`zsh -f -i` and `zshrs -f -i`) on their own
pty, so budget ~5-30s per cell and expect a shrink to spend up to
`--shrink-probes` cells more per new fingerprint.

## Verdicts

`PASS` / `FAIL` are the comparison. `TIMEOUT` means a side ran out of
measurement budget, so the two screens were never both final — counted and
printed separately, re-measured once serially, and never scored as a pass.
`SKIP` means the case's command is not installed on this host, so neither shell
can reach a completer; also never scored as a pass, and ON by default
(`--no-skip-missing` reverts to scoring "both rendered nothing" as a pass).

`--style-fuzz` adds two more, both of which exist so that a config the
REFERENCE shell will not accept can never be dressed up as evidence:

* `INVALID-CONFIG` — zsh's own `zstyle` refused the generated statement at
  definition time (an invalid context pattern). The cell is not run at all,
  because comparing two shells on a config neither can hold says nothing. This
  is a bug in the generator, not in zshrs.
* `REF-REFUSED` — the statement parsed, but the reference zsh complained about
  the VALUE at completion time (an unknown match-specification character, an
  unterminated character class, a completer that does not exist). The cell IS
  still run and still compared — zshrs is required to refuse it identically —
  but it is tallied apart from the clean passes.

Only a `PASS` under a config zsh accepted is evidence of parity, and every
other category keeps the exit status non-zero.
