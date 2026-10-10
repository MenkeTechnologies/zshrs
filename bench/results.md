# Shell benchmark results

Generated 2026-10-10 06:05:58 UTC on Darwin 25.6.0 arm64; hyperfine --warmup 3 --runs 15

| Shell | Binary | Version |
|---|---|---|
| zshrs | `/Users/wizard/RustroverProjects/zshrs/target/release/zshrs` | zshrs 0.13.27 |
| zsh | `/opt/homebrew/bin/zsh` | zsh 5.9.2 (aarch64-apple-darwin25.4.0) |
| fish | `/opt/homebrew/bin/fish` | fish, version 4.9.3 |
| nu | `/opt/homebrew/bin/nu` | 0.116.1 |
| bash | `/opt/homebrew/bin/bash` | GNU bash, version 5.3.20(1)-release (aarch64-apple-darwin25.6.0) |
| ksh | `/opt/homebrew/bin/ksh` |   version         sh (AT&T Research) 93u+m/1.0.10 2024-08-01 |
| dash | `/opt/homebrew/bin/dash` | - |

## Startup (no-op)

| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `zshrs` | 7.4 ± 0.7 |  |  |
| `zsh` | 4.9 ± 0.5 | -34.1% | (1.5x faster) |
| `fish` | 4.4 ± 0.5 | -41.4% | (1.7x faster) |
| `nu` | 8.5 ± 0.3 | +13.9% |  |
| `bash` | 4.1 ± 0.3 | -44.3% | (1.8x faster) |
| `ksh` | 4.0 ± 0.1 | -46.2% | (1.9x faster) |
| `dash` | 1.8 ± 0.1 | -76.3% | (4.2x faster) |

## Arithmetic loop (100000 iterations)

| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `zshrs` | 196.8 ± 7.4 |  |  |
| `zsh` | 124.6 ± 4.9 | -36.7% | (1.6x faster) |
| `fish` | 1785.2 ± 45.6 | +807.1% | (9.1x slower) |
| `nu` | 48.4 ± 1.1 | -75.4% | (4.1x faster) |
| `bash` | 152.8 ± 6.3 | -22.4% |  |
| `ksh` | 49.9 ± 2.3 | -74.6% | (3.9x faster) |
| `dash` | 153.0 ± 3.9 | -22.3% |  |

## Function calls (20000)

| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `zshrs` | 133.8 ± 6.1 |  |  |
| `zsh` | 113.1 ± 10.4 | -15.5% |  |
| `fish` | 371.8 ± 30.7 | +177.8% | (2.8x slower) |
| `nu` | 27.0 ± 0.5 | -79.8% | (5.0x faster) |
| `bash` | 58.9 ± 3.5 | -56.0% | (2.3x faster) |
| `ksh` | 17.3 ± 0.6 | -87.1% | (7.7x faster) |
| `dash` | 50.5 ± 0.5 | -62.2% | (2.6x faster) |

## String append (5000)

| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `zshrs` | 30.7 ± 7.1 |  |  |
| `zsh` | 52.2 ± 1.4 | +70.2% | (1.7x slower) |
| `fish` | 208.7 ± 15.6 | +579.7% | (6.8x slower) |
| `nu` | 27.2 ± 1.5 | -11.4% |  |
| `bash` | 70.0 ± 3.6 | +128.1% | (2.3x slower) |
| `ksh` | 11.3 ± 0.2 | -63.2% | (2.7x faster) |
| `dash` | 16.5 ± 0.2 | -46.3% | (1.9x faster) |

## Command substitution (500)

| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `zshrs` | 86.4 ± 8.3 |  |  |
| `zsh` | 279.3 ± 10.8 | +223.2% | (3.2x slower) |
| `fish` | 23.4 ± 1.9 | -72.9% | (3.7x faster) |
| `nu` | 763.1 ± 71.2 | +783.1% | (8.8x slower) |
| `bash` | 301.8 ± 12.6 | +249.2% | (3.5x slower) |
| `ksh` | 6.8 ± 0.3 | -92.1% | (12.6x faster) |
| `dash` | 255.1 ± 13.3 | +195.2% | (3.0x slower) |

## External command spawn (200 x /usr/bin/true)

| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `zshrs` | 261.8 ± 18.2 |  |  |
| `zsh` | 401.4 ± 33.0 | +53.3% | (1.5x slower) |
| `fish` | 241.3 ± 15.7 | -7.8% |  |
| `nu` | 302.4 ± 18.2 | +15.5% |  |
| `bash` | 359.0 ± 20.1 | +37.1% | (1.4x slower) |
| `ksh` | 251.5 ± 21.9 | -3.9% |  |
| `dash` | 348.2 ± 16.1 | +33.0% | (1.3x slower) |

## Pipeline (seq 200000 | sort -n | uniq | wc -l)

| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `zshrs` | 59.9 ± 3.5 |  |  |
| `zsh` | 55.5 ± 2.2 | -7.3% |  |
| `fish` | 55.6 ± 2.7 | -7.1% |  |
| `nu` | 58.8 ± 3.0 | -1.8% |  |
| `bash` | 55.3 ± 2.7 | -7.6% |  |
| `ksh` | 60.8 ± 2.7 | +1.6% |  |
| `dash` | 54.2 ± 2.1 | -9.6% |  |

## Glob (src/*/*.rs)

| Command | Mean Wall Time [ms] | Change | Factor |
|:---|---:|---:|:---|
| `zshrs` | 7.8 ± 0.5 |  |  |
| `zsh` | 4.8 ± 0.3 | -38.3% | (1.6x faster) |
| `fish` | 4.4 ± 0.2 | -43.1% | (1.8x faster) |
| `nu` | 8.9 ± 0.3 | +14.8% |  |
| `bash` | 5.3 ± 0.3 | -31.7% | (1.5x faster) |
| `ksh` | 4.8 ± 0.2 | -38.4% | (1.6x faster) |
| `dash` | 2.5 ± 0.4 | -67.4% | (3.1x faster) |

