# zshrs JetBrains Plugin

JetBrains IDE support for [zshrs](https://github.com/MenkeTechnologies/zshrs) —
the first compiled Unix shell (Rust bytecode VM + fusevm JIT + persistent
worker pool).

## Features

### Editor
- **File association** for `.zsh` and the standard rc dotfiles (`.zshrc`,
  `.zshenv`, `.zlogin`, `.zlogout`, `.zprofile`, `.zpreztorc`).
- **Hand-rolled lexer** tokenizing comments, shebangs, single / double /
  ANSI-C / backtick strings, heredocs (`<<EOF`, `<<-EOF`, `<<'EOF'`),
  integers, reserved-word keywords (`if`/`then`/`else`/`elif`/`fi`/`for`/
  `while`/`do`/`done`/`case`/`esac`/`select`/`repeat`), declaration keywords
  (`local`/`typeset`/`declare`/`export`/`readonly`/`integer`/`float`),
  modifier keywords (`alias`/`setopt`/`zstyle`/`zmodload`/`autoload`/
  `bindkey`/`compdef`/`zcompile`), I/O keywords (`source`/`eval`/`exec`/
  `echo`/`print`/`printf`/`read`/`trap`), zsh builtins (`cd`/`pwd`/`hash`/
  `unhash`/`fc`/`compinit`/...), sigil variables (`$name`, `${name}`,
  `${(P)var}`), special variables (`$0`/`$?`/`$!`/`$$`/`$#`/`$*`/`$@`/`$-`/`$_`),
  pipes (`|`, `|&`), redirects (`>`, `>>`, `<`, `<<`, `<<<`, `&>`, `2>&1`),
  logical operators (`&&`, `||`), backgrounding (`&`), glob characters
  (`*`, `?`, `[…]`), assignment forms (`=`, `+=`, `?=`, `:=`).
- **42 color slots** under *Settings → Editor → Color Scheme → zshrs* —
  every token category is independently themeable with stable `ZSHRS_*`
  `TextAttributesKey` names.
- **Comments**:
  - `#` line comments — Cmd/Ctrl-`/`
  - `: <<'###BLOCK###' … ###BLOCK###` block wrapper — Cmd/Ctrl-Opt-`/`

### LSP
- LSP client wired to `zshrs --lsp` over stdio. Server capabilities (as
  provided by the in-tree `src/extensions/lsp.rs`):
  - `completion` for builtins, options, parameter names, function names,
    keywords (trigger characters `$`, `{`, `-`, `:`, all letters)
  - `hover` cards for builtins / keywords / options
  - `definition` / `references` / `documentHighlight` for function names
    inside the current document
  - `rename` with prepare
  - `documentSymbol` — function declarations and top-level aliases
  - `semanticTokens` (full document, matching the lexer's token classes)
  - `foldingRange` — fold every `{ … }` / `do … done` / `case … esac` block
    plus 3+ consecutive `#`-line comment runs
  - `formatting` — Cmd/Ctrl-Opt-`L` runs the file through `zshrs --fmt`
  - `publishDiagnostics` (parse errors with line/col from
    `src/ported/lex.rs` + `parse.rs`)

### Run / Debug
- **Run configurations** with toggles for `-f`/`--no-rcs`, `-x` (xtrace),
  `-v` (verbose), `--disasm` (fusevm bytecode disassembly), `--dump-ast`,
  and a compat-mode selector (`zsh` / `bash` / `ksh` / `posix` / default
  `zshrs`).
- **Context-menu *Run with zshrs*** on any `.zsh` file in the editor or
  project view; auto-creates a run config.
- **Debugger** (DAP-backed, TCP socket):
  - Line breakpoints from the gutter (toggle, enable/disable)
  - Continue / Step Over / Step Into / Step Out / Pause / Run to Cursor
  - **Frames** with file:line per frame, source navigation
  - **Variables panel** — scalars, arrays (`@arr`), and associative arrays
    (`%hash`) all rendered, expandable on click
  - **Evaluate** dialog — `${var}`, command substitutions `$(cmd)`, and
    arithmetic `$(( expr ))` resolved against the paused frame
  - **Console** streams the program's `print`/`echo`/`printf` output in
    real time (DAP `output` events merged with process stdio)

### Reflection tool window
- *View → Tool Windows → zshrs* (right edge).
- Tabs populated from `zshrs --dump-reflection`:
  - `builtins`, `keywords`, `options` (zsh setopt names),
    `parameters` (`PARAMETER_FLAGS`), `redirects`, `aliases`,
    `special_vars`
- Per-tab search field filters across name + category.
- **Left-click on any leaf → docs popup** anchored at the click. Renders
  `zshrs --docs <name>` with ANSI colors interpreted via IntelliJ's
  `AnsiEscapeDecoder`.

## Requirements

- A paid JetBrains IDE on **2024.2+** (RustRover, IDEA Ultimate, GoLand,
  PyCharm Pro, WebStorm, RubyMine, PhpStorm, CLion, Rider, DataGrip, Aqua).
  The LSP API isn't in Community editions, so the plugin won't load there.
- The `zshrs` binary on `$PATH`, or configured under *Settings → Tools →
  zshrs → zshrs executable*.

## Building

```sh
cd editors/intellij
export JAVA_HOME=$(/usr/libexec/java_home -v 17)   # macOS; or set to any JDK 17 install
./gradlew buildPlugin             # produces build/distributions/zshrs-intellij-<v>.zip
./gradlew runIde                  # launches a sandbox IDE with the plugin installed
./gradlew verifyPlugin            # plugin verifier against recommended IDE matrix
```

**JDK 17 is required** — Kotlin 2.0.21 (pinned by the IntelliJ Platform
Gradle Plugin 2.16) crashes parsing the 3-part version string of newer
JDKs (`25.0.2`, `26.0.1`, …) inside `JavaVersion.parse`. Set `JAVA_HOME`
to a JDK 17 install before running gradle. The plugin itself targets
JVM 17, so any IDE on 2024.2+ runs it.

First build downloads the IntelliJ Platform SDK (~1 GB), takes a few
minutes, and is cached under `editors/intellij/.intellijPlatform/` (which
is gitignored).

## Installing

1. *Settings → Plugins → ⚙ → Install Plugin from Disk…*
2. Pick `build/distributions/zshrs-intellij-<version>.zip`.
3. Restart the IDE.
4. Open any `.zsh` file. The LSP starts automatically; the debugger
   activates when you click Debug.

## Configuration

*Settings → Tools → zshrs*:

| Section     | Setting                              | Default              |
|-------------|--------------------------------------|----------------------|
| Interpreter | zshrs executable                     | first `zshrs` on `$PATH` |
| LSP         | Enable LSP                           | on                   |
| LSP         | Extra LSP args                       | empty                |
| LSP         | LSP environment (`KEY=VAL`)          | empty                |
| LSP         | Auto-restart LSP on settings change  | on                   |
| LSP         | Show builtin hovers                  | on                   |
| LSP         | Log LSP traffic to file              | off                  |
| Editor      | Disable lexer highlighting           | off                  |
| Editor      | File extensions                      | `zsh`                |
| Run configs | Default new configs to `-f`/`--no-rcs` | off                |

Color scheme entries: *Settings → Editor → Color Scheme → zshrs* (42
sub-categories grouped under Comments / Strings / Numbers / Keywords /
Names / Variables / Operators / Punctuation / Errors).

## How the debugger works

Plugin side (`com.menketechnologies.zshrs.dap`):
1. `ZshrsDebugRunner.doExecute` opens a `ServerSocket(0)` on `127.0.0.1`.
2. Spawns `zshrs --dap 127.0.0.1:<port>` via `KillableColoredProcessHandler`.
3. Waits up to 10 s for zshrs to connect back; then runs DAP over that
   socket. Program stdio stays on the process handler for the Console.
4. `ZshrsDebugProcess` builds an `XDebugSession` and on every `stopped`
   event synchronously fetches frames + scopes + variables, hands them
   to `ZshrsStackFrame` with pre-populated children — avoids the
   empty-variables symptom of IntelliJ 2026.1's split-debugger async
   expansion.
5. `ZshrsDapClient` reads Content-Length-framed JSON-RPC as raw BYTES
   (not chars), so multi-byte UTF-8 in variable reprs never desyncs the
   framing.

zshrs side (`src/extensions/dap.rs`):
- DAP server listens on the TCP port. Speaks: `initialize`, `launch`,
  `setBreakpoints`, `configurationDone`, `threads`, `stackTrace`,
  `scopes`, `variables`, `continue`, `next`, `stepIn`, `stepOut`, `pause`,
  `evaluate`, `disconnect`.
- Hooks into the shell evaluator at statement boundaries to honour
  breakpoints and step modes; pulls locals/globals from the current
  scope chain for the variables view.

## Plugin architecture

```
editors/intellij/
├── build.gradle.kts                          # IntelliJ Platform Gradle Plugin 2.16
├── gradle.properties                         # platform version, plugin version, JVM
├── settings.gradle.kts
└── src/main/
    ├── kotlin/com/menketechnologies/zshrs/
    │   ├── ZshrsLanguage.kt                  # Language singleton
    │   ├── ZshrsFileType.kt                  # .zsh + dotfiles → zshrs
    │   ├── ZshrsIcons.kt                     # icon loader
    │   ├── ZshrsColors.kt                    # 42 ZSHRS_* TextAttributesKey constants
    │   ├── ZshrsTokenTypes.kt                # token type enum
    │   ├── ZshrsLexer.kt                     # hand-rolled zsh lexer
    │   ├── ZshrsSyntaxHighlighter.kt         # token → color mapping
    │   ├── ZshrsColorSettingsPage.kt         # IDE color-scheme entries
    │   ├── ZshrsCommenter.kt                 # `#` line comments
    │   ├── ZshrsSettings.kt                  # persistent settings
    │   ├── ZshrsSettingsConfigurable.kt
    │   ├── lsp/
    │   │   ├── ZshrsLspServerSupportProvider.kt
    │   │   └── ZshrsLspServerDescriptor.kt
    │   ├── run/
    │   │   ├── ZshrsRunConfigurationType.kt
    │   │   ├── ZshrsRunConfigurationOptions.kt
    │   │   ├── ZshrsRunConfiguration.kt
    │   │   ├── ZshrsRunConfigurationEditor.kt
    │   │   ├── ZshrsRunConfigurationProducer.kt
    │   │   ├── ZshrsProgramRunner.kt         # Run executor
    │   │   └── ZshrsDebugRunner.kt           # Debug executor (DAP)
    │   ├── dap/
    │   │   ├── ZshrsDapClient.kt             # byte-based DAP protocol client
    │   │   ├── ZshrsDebugProcess.kt          # XDebugProcess
    │   │   ├── ZshrsDebuggerEditorsProvider.kt
    │   │   ├── ZshrsBreakpointType.kt        # xdebugger.breakpointType
    │   │   ├── ZshrsBreakpointHandler.kt
    │   │   ├── ZshrsStackFrame.kt
    │   │   ├── ZshrsSuspendContext.kt
    │   │   ├── ZshrsValue.kt                 # XValue with recursive children
    │   │   └── ZshrsEvaluator.kt             # Evaluate dialog backend
    │   ├── toolwindow/
    │   │   └── ZshrsReflectionToolWindow.kt
    │   └── actions/
    │       └── RunZshrsFileAction.kt
    └── resources/
        ├── META-INF/plugin.xml
        └── icons/zshrs.svg
```

The Rust side lives in:
- `src/extensions/lsp.rs` — LSP server (`zshrs --lsp`)
- `src/extensions/dap.rs` — DAP server (`zshrs --dap HOST:PORT`)

Both modules pull tokens from `src/ported/lex.rs` and AST from
`src/ported/parse.rs` for diagnostics and symbol extraction.

## Version compatibility

Plugin version tracks the zshrs Cargo workspace version. `gradle.properties`
controls the supported IDE range via `pluginSinceBuild` / `pluginUntilBuild`.
Currently targets `2024.2.4` SDK against builds `242 .. 261.*`.

## Limitations

- **No PSI tree** — relies entirely on the LSP for symbol navigation.
- **Debugger v1**: no conditional or hit-count breakpoints, no watch
  expressions, no Set Value, single-thread only.
- **Lexer** is approximate for arithmetic-expansion `$(( ))` and complex
  nested expansions; server-side semantic tokens fill in where the lexer
  is wrong.

## License

MIT, same as zshrs.
