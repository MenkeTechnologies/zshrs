# Recursive glob benchmark results

Generated 2026-10-13 21:24:48 UTC on Darwin 25.6.0 arm64, 18 logical cores; hyperfine --warmup 3 --runs 15

Tree: 30000 files in 1100 directories (100 x 10 leaf directories, 30 files each).

| Shell | Binary | Version |
|---|---|---|
| zsh | `/opt/homebrew/bin/zsh` | zsh 5.9.2 (aarch64-apple-darwin25.4.0) |
| zshrs | `/Users/wizard/RustroverProjects/zshrs/target/release/zshrs` | zshrs 0.13.34 |

## `**/*` (every file and directory)

| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `zsh` | 131.3 ± 8.4 |  |  |
| `zshrs (parallel)` | 104.7 ± 2.0 | -20.2% |  |
| `zshrs (serial)` | 151.0 ± 27.8 | +15.0% |  |

## `**/*.rs` (extension filter)

| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `zsh` | 89.1 ± 2.9 |  |  |
| `zshrs (parallel)` | 58.4 ± 2.5 | -34.4% | (1.5x faster) |
| `zshrs (serial)` | 99.6 ± 8.3 | +11.9% |  |

## `**/*(.)` (plain files, qualifier)

| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `zsh` | 145.4 ± 6.9 |  |  |
| `zshrs (parallel)` | 122.9 ± 2.6 | -15.5% |  |
| `zshrs (serial)` | 160.9 ± 8.7 | +10.7% |  |

