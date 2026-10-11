# Shell benchmark results

Generated 2026-10-10 13:42:51 UTC on Darwin 25.6.0 arm64; hyperfine --warmup 3 --runs 15

| Shell | Binary | Version |
|---|---|---|
| zshrs | `/Users/wizard/RustroverProjects/zshrs/target/release/zshrs` | zshrs 0.13.27 |
| zsh | `/opt/homebrew/bin/zsh` | zsh 5.9.2 (aarch64-apple-darwin25.4.0) |
| fish | `/opt/homebrew/bin/fish` | fish, version 4.9.3 |
| nu | `/opt/homebrew/bin/nu` | 0.116.1 |
| bash | `/opt/homebrew/bin/bash` | GNU bash, version 5.3.20(1)-release (aarch64-apple-darwin25.6.0) |
| ksh | `/opt/homebrew/bin/ksh` |   version         sh (AT&T Research) 93u+m/1.0.10 2024-08-01 |
| mksh | `/opt/homebrew/bin/mksh` | @(#)MIRBSD KSH R59 2020/10/31 |
| dash | `/opt/homebrew/bin/dash` | - |

## Startup (no-op)

| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `zshrs` | 7.4 ± 0.4 |  |  |
| `zsh` | 4.6 ± 0.8 | -37.7% | (1.6x faster) |
| `fish` | 5.9 ± 0.6 | -21.0% |  |
| `nu` | 11.0 ± 0.4 | +48.4% | (1.5x slower) |
| `bash` | 6.5 ± 0.3 | -12.3% |  |
| `ksh` | 5.5 ± 0.3 | -26.3% |  |
| `mksh` | 2.3 ± 0.2 | -68.8% | (3.2x faster) |
| `dash` | 2.3 ± 0.2 | -68.6% | (3.2x faster) |

## Arithmetic loop (100000 iterations)

| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `zshrs` | 178.4 ± 8.4 |  |  |
| `zsh` | 122.3 ± 6.0 | -31.5% | (1.5x faster) |
| `fish` | 1626.5 ± 52.5 | +811.7% | (9.1x slower) |
| `nu` | 48.0 ± 2.5 | -73.1% | (3.7x faster) |
| `bash` | 150.8 ± 5.4 | -15.5% |  |
| `ksh` | 47.5 ± 2.6 | -73.4% | (3.8x faster) |
| `mksh` | 104.0 ± 3.4 | -41.7% | (1.7x faster) |
| `dash` | 150.4 ± 2.8 | -15.7% |  |

## Function calls (20000)

| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `zshrs` | 124.8 ± 5.6 |  |  |
| `zsh` | 107.6 ± 8.2 | -13.8% |  |
| `fish` | 353.8 ± 31.2 | +183.5% | (2.8x slower) |
| `nu` | 26.8 ± 0.8 | -78.5% | (4.7x faster) |
| `bash` | 59.2 ± 3.1 | -52.5% | (2.1x faster) |
| `ksh` | 16.9 ± 0.5 | -86.4% | (7.4x faster) |
| `mksh` | 31.0 ± 1.0 | -75.2% | (4.0x faster) |
| `dash` | 51.3 ± 1.8 | -58.9% | (2.4x faster) |

## String append (5000)

| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `zshrs` | 23.8 ± 2.0 |  |  |
| `zsh` | 52.4 ± 1.0 | +120.3% | (2.2x slower) |
| `fish` | 200.4 ± 16.6 | +742.6% | (8.4x slower) |
| `nu` | 25.6 ± 1.5 | +7.8% |  |
| `bash` | 71.4 ± 4.6 | +200.1% | (3.0x slower) |
| `ksh` | 11.3 ± 0.5 | -52.7% | (2.1x faster) |
| `mksh` | 87.4 ± 3.0 | +267.3% | (3.7x slower) |
| `dash` | 16.6 ± 0.5 | -30.3% | (1.4x faster) |

## Command substitution (500)

| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `zshrs` | 90.3 ± 13.7 |  |  |
| `zsh` | 268.9 ± 27.7 | +197.7% | (3.0x slower) |
| `fish` | 18.9 ± 1.4 | -79.1% | (4.8x faster) |
| `nu` | 707.2 ± 47.3 | +682.8% | (7.8x slower) |
| `bash` | 281.6 ± 25.4 | +211.7% | (3.1x slower) |
| `ksh` | 5.3 ± 0.1 | -94.1% | (17.0x faster) |
| `mksh` | 209.9 ± 28.4 | +132.4% | (2.3x slower) |
| `dash` | 229.1 ± 14.9 | +153.6% | (2.5x slower) |

## External command spawn (200 x /usr/bin/true)

| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `zshrs` | 226.1 ± 22.6 |  |  |
| `zsh` | 315.4 ± 27.8 | +39.5% | (1.4x slower) |
| `fish` | 214.8 ± 20.6 | -5.0% |  |
| `nu` | 276.9 ± 28.8 | +22.5% |  |
| `bash` | 329.2 ± 39.9 | +45.6% | (1.5x slower) |
| `ksh` | 215.5 ± 30.2 | -4.7% |  |
| `mksh` | 297.0 ± 31.0 | +31.3% | (1.3x slower) |
| `dash` | 295.3 ± 19.6 | +30.6% | (1.3x slower) |

## Pipeline (seq 200000 | sort -n | uniq | wc -l)

| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `zshrs` | 57.1 ± 2.8 |  |  |
| `zsh` | 54.7 ± 4.4 | -4.2% |  |
| `fish` | 54.5 ± 2.8 | -4.5% |  |
| `nu` | 58.9 ± 3.8 | +3.1% |  |
| `bash` | 54.4 ± 3.1 | -4.7% |  |
| `ksh` | 57.8 ± 5.5 | +1.2% |  |
| `mksh` | 54.2 ± 3.3 | -5.0% |  |
| `dash` | 53.0 ± 3.7 | -7.2% |  |

## Glob (src/*/*.rs)

| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `zshrs` | 7.0 ± 0.6 |  |  |
| `zsh` | 3.9 ± 0.1 | -43.8% | (1.8x faster) |
| `fish` | 3.5 ± 0.1 | -49.5% | (2.0x faster) |
| `nu` | 7.7 ± 0.2 | +10.1% |  |
| `bash` | 4.5 ± 0.2 | -35.9% | (1.6x faster) |
| `ksh` | 3.9 ± 0.1 | -44.0% | (1.8x faster) |
| `mksh` | 1.9 ± 0.0 | -73.0% | (3.7x faster) |
| `dash` | 2.0 ± 0.1 | -72.1% | (3.6x faster) |

