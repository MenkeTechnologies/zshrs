# zinit corpus

Verbatim copies of [zdharma-continuum/zinit](https://github.com/zdharma-continuum/zinit) `bin/*.zsh` scripts, used as a real-world stress corpus for the recorder harness in `tests/recorder_harness.rs`.

These are not part of zshrs runtime; they are sourced by `zshrs-recorder --no-daemon --file <path>` during `cargo test --features recorder --test recorder_harness` and the captured event counts are pinned per file.

License: MIT (see `LICENSE`). Source: zinit upstream.

A bump of zinit upstream may shift the pinned counts; if so, re-copy + re-gauge + update the constants in `tests/recorder_harness.rs`.
