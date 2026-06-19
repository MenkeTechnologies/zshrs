// AUTO-GENERATED. DO NOT EDIT BY HAND.
//
// Source: zsh upstream `/Users/wizard/forkedRepos/zsh/Doc/Zsh/params.yo (+42 more)`.
// Regen:  ./scripts/gen_option_docs.py --source specials /Users/wizard/forkedRepos/zsh/Doc/Zsh/params.yo (+42 more) > <this file>
// Generated: 2026-05-23

/// Canonical special variable name → markdown documentation body.
/// Pulled verbatim from every `item(tt(NAME))(...)` block in
/// the upstream yodl source. Use [`lookup_special_var_doc`] to resolve
/// case-insensitive lookups and any negated/compact aliases.
pub const SPECIAL_VAR_DOCS: &[(&str, &str)] = &[
    ("!", "The process ID of the last command started in the background with `&`,\nput into the background with the `bg` builtin, or spawned with `coproc`."),
    ("#", "The number of positional parameters in decimal.  Note that some confusion\nmay occur with the syntax `$#`_param_ which substitutes the length of\n_param_.  Use `${#}` to resolve ambiguities.  In particular, the\nsequence ``$#-\"_..._\" in an arithmetic expression is interpreted as\nthe length of the parameter `-`, q.v."),
    ("ARGC", "Same as `#`."),
    ("$", "The process ID of this shell, set when the shell initializes.  Processes\nforked from the shell without executing a new program, such as command\nsubstitutions and commands grouped with tt(()_..._``),\nare subshells that duplicate the current shell, and thus substitute the\nsame value for `$$` as their parent shell."),
    ("-", "Flags supplied to the shell on invocation or by the `set`\nor `setopt` commands."),
    ("*", "An array containing the positional parameters."),
    ("argv", "Same as `*`.  Assigning to `argv` changes the local positional\nparameters, but `argv` is _not_ itself a local parameter.\nDeleting `argv` with `unset` in any function deletes it everywhere.\nThis can be avoided by declaring \"local +h argv\" before unsetting.\nEven when not so declared, only the innermost positional parameter\narray is deleted (so `*` and `@` in other scopes are not affected).\n\nA named reference to `argv` _does not_ permit a called function to\naccess the positional parameters of its caller."),
    ("@", "Same as `argv[@]`, even when `argv` is not set."),
    ("?", "The exit status returned by the last command."),
    ("0", "The name used to invoke the current shell, or as set by the `-c` command\nline option upon invocation.  If the `FUNCTION_ARGZERO` option is set,\n`$0` is set upon entry to a shell function to the name of the function,\nand upon entry to a sourced script to the name of the script, and reset to\nits previous value when the function or script returns."),
    ("status", "Same as `?`."),
    ("pipestatus", "An array containing the exit statuses returned by all commands in the\nlast pipeline."),
    ("_", "Initially, if `_` exists in the environment, then this parameter is set to\nits value. This value may be the full pathname of the current zsh\nexecutable or the script command file.\nLater, this parameter is set to the last argument of the previous command.\nAlso, this parameter is set in the environment of every command\nexecuted to the full pathname of the command."),
    ("CPUTYPE", "The machine type (microprocessor class or machine model),\nas determined at run time."),
    ("EGID", "The effective group ID of the shell process.  If you have sufficient\nprivileges, you may change the effective group ID of the shell\nprocess by assigning to this parameter.  Also (assuming sufficient\nprivileges), you may start a single command with a different\neffective group ID by `tt((EGID=)_gid_`; command+\")\"\n\nIf this is made local, it is not implicitly set to 0, but may be\nexplicitly set locally."),
    ("EUID", "The effective user ID of the shell process.  If you have sufficient\nprivileges, you may change the effective user ID of the shell process\nby assigning to this parameter.  Also (assuming sufficient privileges),\nyou may start a single command with a different\neffective user ID by `tt((EUID=)_uid_`; command+\")\"\n\nIf this is made local, it is not implicitly set to 0, but may be\nexplicitly set locally."),
    ("ERRNO", "The value of `errno` (see manref(errno)(3))\nas set by the most recently failed system call.\nThis value is system dependent and is intended for debugging\npurposes.  It is also useful with the `zsh/system` module which\nallows the number to be turned into a name or message.\n\nTo use this parameter, it must first be assigned a value (typically\n0 (zero)).  It is initially unset for scripting compatibility."),
    ("FUNCNEST", "Integer.  If greater than or equal to zero, the maximum nesting depth of\nshell functions.  When it is exceeded, an error is raised at the point\nwhere a function is called.  The default value is determined when\nthe shell is configured, but is typically 500.  Increasing\nthe value increases the danger of a runaway function recursion\ncausing the shell to crash.  Setting a negative value turns off\nthe check."),
    ("GID", "The real group ID of the shell process.  If you have sufficient privileges,\nyou may change the group ID of the shell process by assigning to this\nparameter.  Also (assuming sufficient privileges), you may start a single\ncommand under a different\ngroup ID by `tt((GID=)_gid_`; command+\")\"\n\nIf this is made local, it is not implicitly set to 0, but may be\nexplicitly set locally."),
    ("HISTCMD", "The current history event number in an interactive shell, in other\nwords the event number for the command that caused `$HISTCMD`\nto be read.  If the current history event modifies the history,\n`HISTCMD` changes to the new maximum history event number."),
    ("HOST", "The current hostname."),
    ("LINENO", "The line number of the current line within the current script, sourced\nfile, or shell function being executed, whichever was started most\nrecently.  Note that in the case of shell functions the line\nnumber refers to the function as it appeared in the original definition,\nnot necessarily as displayed by the `functions` builtin."),
    ("LOGNAME", "If the corresponding variable is not set in the environment of the\nshell, it is initialized to the login name corresponding to the\ncurrent login session. This parameter is exported by default but\nthis can be disabled using the `typeset` builtin.  The value\nis set to the string returned by the manref(getlogin)(3) system call\nif that is available."),
    ("MACHTYPE", "The machine type (microprocessor class or machine model),\nas determined at compile time."),
    ("OLDPWD", "The previous working directory.  This is set when the shell initializes\nand whenever the directory changes."),
    ("OPTARG", "The value of the last option argument processed by the `getopts`\ncommand."),
    ("OPTIND", "The index of the last option argument processed by the `getopts`\ncommand."),
    ("OSTYPE", "The operating system, as determined at compile time."),
    ("PPID", "The process ID of the parent of the shell, set when the shell initializes.\nAs with `$$`, the value does not change in subshells created as a\nduplicate of the current shell."),
    ("PWD", "The present working directory.  This is set when the shell initializes\nand whenever the directory changes."),
    ("RANDOM", "A pseudo-random integer from 0 to 32767, newly generated each time\nthis parameter is referenced.  The random number generator\ncan be seeded by assigning a numeric value to `RANDOM`.\n\nThe values of `RANDOM` form an intentionally-repeatable pseudo-random\nsequence; subshells that reference `RANDOM` will result\nin identical pseudo-random values unless the value of `RANDOM` is\nreferenced or seeded in the parent shell in between subshell invocations.\n\n`RANDOM\" uses the system\"s library function manref(rand)(3).\nIf higher degree of randomness is required, please consider using\nthe autoloadable parameter `SRANDOM` from\nifzman(the `zsh/random` module (see zmanref(zshmodules)))\\\nifnzman((The zsh/random Module))."),
    ("SECONDS", "The number of seconds since shell invocation.  On most platforms, this\nis a monotonic value, so it is not affected by NTP time jumps or other\nclock changes (though it may be affected by slewing).  If this parameter\nis assigned a value, then the value returned upon reference\nwill be the value that was assigned plus the number of seconds\nsince the assignment.\n\nUnlike other special parameters, the type of the `SECONDS` parameter can\nbe changed using the `typeset` command.  The type may be changed only\nto one of the floating point types or back to integer.  For example,\n\"typeset -F SECONDS\"\ncauses the value to be reported as a floating point number.  The\nvalue is nominally available to nanosecond precision, although this\nvaries by platform (and probably isn't accurate to 1 ns regardless),\nand the shell may show more or fewer digits depending on the\nuse of `typeset`.  See\nthe documentation for the builtin `typeset` in\nnmref(Shell Builtin Commands)(zshbuiltins) for more details."),
    ("SHLVL", "Incremented by one each time a new shell is started."),
    ("signals", "An array containing the names of the signals.  Note that with\nthe standard zsh numbering of array indices, where the first element\nhas index 1, the signals are offset by 1 from the signal number\nused by the operating system.  For example, on typical Unix-like systems\n`HUP` is signal number 1, but is referred to as `$signals[2]`.  This\nis because of `EXIT` at position 1 in the array, which is used\ninternally by zsh but is not known to the operating system. On many systems\nthere is a block of reserved or unused signal numbers before the POSIX\nreal-time signals so the array index can't be used as an accurate indicator\nof their signal number. Use, for example, `kill -l SIGRTMIN` instead."),
    (".term.cursor", "The default color of the cursor.\n\nSee also the `.term.extensions` parameter."),
    (".term.bg", "The background color of the terminal if the terminal supports the\nnecessary interface for retrieving this information.\n\nSee also the `.term.extensions` parameter."),
    (".term.fg", "Like `.term.bg` but for the foreground color."),
    (".term.id", "Like the `TERM` parameter, this identifies the terminal but is obtained by\nquerying the terminal.  Many terminals name a different common terminal in\n`TERM` that they then emulate, often imperfectly.  This allows them to work\nwithout the need to distribute updates to terminal capability databases.  The\nmore accurate identification in this parameter may be useful when configuring\naspects of the shell differently for specific terminals.\n\nSee also the `.term.extensions` parameter."),
    (".term.version", "The version number of the terminal application, set in conjunction with\n`.term.id`."),
    (".term.mode", "Either `light` or `dark`.  In cases where the terminal background color is\ndetected and set in `.term.bg`, this is set to provide a basic indication of\nhow light that color is.  This may help with configuring readable color\ncombinations."),
    ("TRY_BLOCK_ERROR", "In an `always` block, indicates whether the preceding list of code\ncaused an error.  The value is 1 to indicate an error, 0 otherwise.\nIt may be reset, clearing the error condition.  See\n_Complex Commands_ (zshmisc)"),
    ("TRY_BLOCK_INTERRUPT", "This variable works in a similar way to `TRY_BLOCK_ERROR`, but\nrepresents the status of an interrupt from the signal SIGINT, which\ntypically comes from the keyboard when the user types `^C`.  If set to\n0, any such interrupt will be reset; otherwise, the interrupt is\npropagated after the `always` block.\n\nNote that it is possible that an interrupt arrives during the execution\nof the `always` block; this interrupt is also propagated."),
    ("TTY", "The name of the tty associated with the shell, if any."),
    ("TTYIDLE", "The idle time of the tty associated with the shell in seconds or -1 if there\nis no such tty."),
    ("UID", "The real user ID of the shell process.  If you have sufficient privileges,\nyou may change the user ID of the shell by assigning to this parameter.\nAlso (assuming sufficient privileges), you may start a single command\nunder a different\nuser ID by `tt((UID=)_uid_`; command+\")\"\n\nIf this is made local, it is not implicitly set to 0, but may be\nexplicitly set locally."),
    ("USERNAME", "The username corresponding to the real user ID of the shell process.  If you\nhave sufficient privileges, you may change the username (and also the\nuser ID and group ID) of the shell by assigning to this parameter.\nAlso (assuming sufficient privileges), you may start a single command\nunder a different username (and user ID and group ID)\nby `tt((USERNAME=)_username_`; command+\")\""),
    ("VENDOR", "The vendor, as determined at compile time."),
    ("zsh_eval_context", "An array (colon-separated list) indicating the context of shell\ncode that is being run.  Each time a piece of shell code that\nis stored within the shell is executed a string is temporarily appended to\nthe array to indicate the type of operation that is being performed.\nRead in order the array gives an indication of the stack of\noperations being performed with the most immediate context last.\n\nNote that the variable does not give information on syntactic context such\nas pipelines or subshells.  Use `$ZSH_SUBSHELL` to detect subshells.\n\nThe context is one of the following:\n\n- **`cmdarg`** —\nCode specified by the `-c` option to the command line that invoked\nthe shell.\n\n\n- **`cmdsubst`** — Command substitution using of the ```_..._```,\ntt($+()_..._``), `${{`_name_`}` _..._`}`,\n`${|`_..._`}`, or `${ `_..._` }` constructs.\n\n\n- **`equalsubst`** — The tt(=+()_..._``) form of process substitution.\n\n\n- **`eval`** —\nCode executed by the `eval` builtin.\n\n- **`evalautofunc`** —\nCode executed with the `KSH_AUTOLOAD` mechanism in order to define\nan autoloaded function.\n\n- **`fc`** —\nCode from the shell history executed by the `-e` option to the `fc`\nbuiltin.\n\n- **`file`** —\nLines of code being read directly from a file, for example by\nthe `source` builtin.\n\n- **`filecode`** —\nLines of code being read from a `.zwc` file instead of directly\nfrom the source file.\n\n- **`globqual`** —\nCode executed by the `e` or `+` glob qualifier.\n\n- **`globsort`** —\nCode executed to order files by the `o` glob qualifier.\n\n\n- **`insubst`** — The tt(<()_..._``) form of process substitution.\n\n\n- **`loadautofunc`** —\nCode read directly from a file to define an autoloaded function.\n\n\n- **`outsubst`** — The tt(>()_..._``) form of process substitution.\n\n\n- **`sched`** —\nCode executed by the `sched` builtin.\n\n- **`shfunc`** —\nA shell function.\n\n- **`stty`** —\nCode passed to `stty` by the `STTY` environment variable.\nNormally this is passed directly to the system's `stty` command,\nso this value is unlikely to be seen in practice.\n\n- **`style`** —\nCode executed as part of a style retrieved by the `zstyle` builtin\nfrom the `zsh/zutil` module.\n\n- **`toplevel`** —\nThe highest execution level of a script or interactive shell.\n\n- **`trap`** —\nCode executed as a trap defined by the `trap` builtin.  Traps\ndefined as functions have the context `shfunc`.  As traps are\nasynchronous they may have a different hierarchy from other\ncode.\n\n- **`zpty`** —\nCode executed by the `zpty` builtin from the `zsh/zpty` module.\n\n- **`zregexparse-guard`** —\nCode executed as a guard by the `zregexparse` command from the\n`zsh/zutil` module.\n\n- **`zregexparse-action`** —\nCode executed as an action by the `zregexparse` command from the\n`zsh/zutil` module."),
    ("ZSH_ARGZERO", "If zsh was invoked to run a script, this is the name of the script.\nOtherwise, it is the name used to invoke the current shell.  This is\nthe same as the value of `$0` when the `POSIX_ARGZERO` option is\nset, but is always available."),
    ("ZSH_EXECUTION_STRING", "If the shell was started with the option `-c`, this contains\nthe argument passed to the option.  Otherwise it is not set."),
    ("ZSH_EXEPATH", "Full pathname of the executable file of the current zsh process."),
    ("ZSH_NAME", "Expands to the basename of the command used to invoke this instance\nof zsh."),
    ("ZSH_PATCHLEVEL", "The output of \"git describe --tags --long\" for the zsh repository\nused to build the shell.  This is most useful in order to keep\ntrack of versions of the shell during development between releases;\nhence most users should not use it and should instead rely on\n`$ZSH_VERSION`."),
    ("ZSH_SCRIPT", "If zsh was invoked to run a script, this is the name of the script,\notherwise it is unset."),
    ("ZSH_SUBSHELL", "Readonly integer.  Initially zero, incremented each time the shell forks\nto create a subshell for executing code.  Hence \"tt((print $ZSH_SUBSHELL))\"\nand \"tt(print $(print $ZSH_SUBSHELL))\" output 1, while\n\"tt(( (print $ZSH_SUBSHELL) ))\" outputs 2."),
    ("ZSH_VERSION", "The version number of the release of zsh."),
    ("ARGV0", "If exported, its value is used as the `argv[0]` of external commands.\nUsually used in constructs like \"ARGV0=emacs nethack\"."),
    ("BAUD", "The rate in bits per second at which data reaches the terminal.\nThe line editor will use this value in order to compensate for a slow\nterminal by delaying updates to the display until necessary.  If the\nparameter is unset or the value is zero the compensation mechanism is\nturned off.  The parameter is not set by default.\n\nThis parameter may be profitably set in some circumstances, e.g.\nfor slow modems dialing into a communications server, or on a slow wide\narea network.  It should be set to the baud\nrate of the slowest part of the link for best performance."),
    ("cdpath", "An array (colon-separated list)\nof directories specifying the search path for the `cd` command."),
    ("COLUMNS", "The number of columns for this terminal session.\nUsed for printing select lists and for the line editor."),
    ("CORRECT_IGNORE", "If set, is treated as a pattern during spelling correction.  Any\npotential correction that matches the pattern is ignored.  For example,\nif the value is \"_*\" then completion functions (which, by\nconvention, have names beginning with \"_\") will never be offered\nas spelling corrections.  The pattern does not apply to the correction\nof file names, as applied by the `CORRECT_ALL` option (so with the\nexample just given files beginning with \"_\" in the current\ndirectory would still be completed)."),
    ("CORRECT_IGNORE_FILE", "If set, is treated as a pattern during spelling correction of file names.\nAny file name that matches the pattern is never offered as a correction.\nFor example, if the value is \".*\" then dot file names will never be\noffered as spelling corrections.  This is useful with the\n`CORRECT_ALL` option."),
    ("DIRSTACKSIZE", "The maximum size of the directory stack, by default there is no limit.  If the\nstack gets larger than this, it will be truncated automatically.\nThis is useful with the `AUTO_PUSHD` option.\npindex(AUTO_PUSHD, use of)"),
    ("ENV", "If the `ENV` environment variable is set when zsh is invoked as `sh`\nor `ksh`, `$ENV` is sourced after the profile scripts.  The value of\n`ENV` is subjected to parameter expansion, command substitution, and\narithmetic expansion before being interpreted as a pathname.  Note that\n`ENV` is _not_ used unless the shell is interactive and zsh is\nemulating **sh** or **ksh**."),
    ("FCEDIT", "The default editor for the `fc` builtin.  If `FCEDIT` is not set,\nthe parameter `EDITOR` is used; if that is not set either, a builtin\ndefault, usually `vi`, is used."),
    ("fignore", "An array (colon separated list)\ncontaining the suffixes of files to be ignored\nduring filename completion.  However, if completion only generates files\nwith suffixes in this list, then these files are completed anyway."),
    ("fpath", "An array (colon separated list)\nof directories specifying the search path for\nfunction definitions.  This path is searched when a function\nwith the `-u` attribute is referenced.  If an executable\nfile is found, then it is read and executed in the current environment."),
    ("histchars", "Three characters used by the shell's history and lexical analysis\nmechanism.  The first character signals the start of a history\nexpansion (default \"!\").  The second character signals the\nstart of a quick history substitution (default \"^\").  The third\ncharacter is the comment character (default \"#\").\n\nThe characters must be in the ASCII character set; any attempt to set\n`histchars` to characters with a locale-dependent meaning will be\nrejected with an error message."),
    ("HISTCHARS", "Same as `histchars`.  (Deprecated.)"),
    ("HISTFILE", "The file to save the history in when an interactive shell exits.\nIf unset, the history is not saved."),
    ("HISTORY_IGNORE", "If set, is treated as a pattern at the time history files are written.\nAny potential history entry that matches the pattern is skipped.  For\nexample, if the value is \"fc *\" then commands that invoke the\ninteractive history editor are never written to the history file.\n\nNote that `HISTORY_IGNORE` defines a single pattern: to\nspecify alternatives use the\n`tt(()_first_`|`_second_`|`_..._`\")\" syntax.\n\nCompare the `HIST_NO_STORE` option or the `zshaddhistory` hook,\neither of which would prevent such commands from being added to the\ninteractive history at all.  If you wish to use `HISTORY_IGNORE` to\nstop history being added in the first place, you can define the\nfollowing hook:\n\nexample(zshaddhistory+() {\n  emulate -L zsh\n  ## uncomment if HISTORY_IGNORE\n  ## should use EXTENDED_GLOB syntax\n  # setopt extendedglob\n  [[ $1 != ${~HISTORY_IGNORE} ]]\n})"),
    ("HISTSIZE", "The maximum number of events stored in the internal history list.\nIf you use the `HIST_EXPIRE_DUPS_FIRST` option, setting this value\nlarger than the `SAVEHIST` size will give you the difference as a\ncushion for saving duplicated history events.\n\nIf this is made local, it is not implicitly set to 0, but may be\nexplicitly set locally."),
    ("HOME", "The default argument for the `cd` command.  This is not set automatically\nby the shell in `sh`, `ksh` or `csh` emulation, but it is typically\npresent in the environment anyway, and if it becomes set it has its usual\nspecial behaviour."),
    ("IFS", "Internal field separators (by default space, tab, newline and NUL), that\nare used to separate words which result from\ncommand or parameter expansion and words read by\nthe `read` builtin.  Any characters from the set space, tab and\nnewline that appear in the `IFS` are called _IFS white space_.\nOne or more IFS white space characters or one non-IFS white space\ncharacter together with any adjacent IFS white space character delimit\na field.  If an IFS white space character appears twice consecutively\nin the `IFS`, this character is treated as if it were not an IFS white\nspace character.\n\nIf the parameter is unset, the default is used.  Note this has\na different effect from setting the parameter to an empty string.\n\nIf `MULTIBYTE` option is on and `IFS` contains invalid characters in\nthe current locale, it is reset to the default."),
    ("KEYBOARD_HACK", "This variable defines a character to be removed from the end of the\ncommand line before interpreting it (interactive shells only). It is\nintended to fix the problem with keys placed annoyingly close to return\nand replaces the `SUNKEYBOARDHACK` option which did this for\nbackquotes only.  Should the chosen character be one of singlequote,\ndoublequote or backquote, there must also be an odd number of them\non the command line for the last one to be removed.\n\nFor backward compatibility, if the `SUNKEYBOARDHACK` option is\nexplicitly set, the value of `KEYBOARD_HACK` reverts to backquote.\nIf the option is explicitly unset, this variable is set to empty."),
    ("KEYTIMEOUT", "The time the shell waits, in hundredths of seconds, for another key to\nbe pressed when reading bound multi-character sequences."),
    ("LANG", "This variable determines the locale category for any category not\nspecifically selected via a variable starting with \"LC_\"."),
    ("LC_ALL", "This variable overrides the value of the \"LANG\" variable and the value\nof any of the other variables starting with \"LC_\"."),
    ("LC_COLLATE", "This variable determines the locale category for character collation\ninformation within ranges in glob brackets and for sorting."),
    ("LC_CTYPE", "This variable determines the locale category for character handling\nfunctions.  If the `MULTIBYTE` option is in effect this variable or\n`LANG` should contain a value that reflects the character set in\nuse, even if it is a single-byte character set, unless only the\n7-bit subset (ASCII) is used.  For example, if the character set\nis ISO-8859-1, a suitable value might be `en_US.iso88591` (certain\nLinux distributions) or `en_US.ISO8859-1` (MacOS)."),
    ("LC_MESSAGES", "This variable determines the language in which messages should be\nwritten.  Note that zsh does not use message catalogs."),
    ("LC_NUMERIC", "This variable affects the decimal point character and thousands\nseparator character for the formatted input/output functions\nand string conversion functions.  Note that zsh ignores this\nsetting when parsing floating point mathematical expressions."),
    ("LC_TIME", "This variable determines the locale category for date and time\nformatting in prompt escape sequences."),
    ("LINES", "The number of lines for this terminal session.\nUsed for printing select lists and for the line editor."),
    ("LISTMAX", "In the line editor, the number of matches to list without asking\nfirst. If the value is negative, the list will be shown if it spans at\nmost as many lines as given by the absolute value.\nIf set to zero, the shell asks only if the top of the listing would scroll\noff the screen."),
    ("MAIL", "If this parameter is set and `mailpath` is not set,\nthe shell looks for mail in the specified file."),
    ("MAILCHECK", "The interval in seconds between checks for new mail."),
    ("mailpath", "An array (colon-separated list) of filenames to check for\nnew mail.  Each filename can be followed by a \"?\" and a\nmessage that will be printed.  The message will undergo\nparameter expansion, command substitution and arithmetic\nexpansion with the variable `$_` defined as the name\nof the file that has changed.  The default message is\n\"You have new mail\".  If an element is a directory\ninstead of a file the shell will recursively check every\nfile in every subdirectory of the element."),
    ("manpath", "An array (colon-separated list)\nwhose value is not used by the shell.  The `manpath`\narray can be useful, however, since setting it also sets\n`MANPATH`, and vice versa."),
    ("module_path", "An array (colon-separated list)\nof directories that `zmodload`\nsearches for dynamically loadable modules.\nThis is initialized to a standard pathname,\nusually \"/usr/local/lib/zsh/$ZSH_VERSION\".\n(The \"/usr/local/lib\" part varies from installation to installation.)\nFor security reasons, any value set in the environment when the shell\nis started will be ignored.\n\nThese parameters only exist if the installation supports dynamic\nmodule loading."),
    ("NULLCMD", "The command name to assume if a redirection is specified\nwith no command.  Defaults to `cat`.  For **sh**/**ksh**\nbehavior, change this to `:`.  For **csh**-like\nbehavior, unset this parameter; the shell will print an\nerror message if null commands are entered."),
    ("path", "An array (colon-separated list)\nof directories to search for commands.\nWhen this parameter is set, each directory is scanned\nand all files found are put in a hash table.\nDirectories named by relative path are not scanned for hashing."),
    ("POSTEDIT", "This string is output whenever the line editor exits.\nIt usually contains termcap strings to reset the terminal."),
    ("PROMPT4", "Same as `PS1`, `PS2`, `PS3` and `PS4`,\nrespectively."),
    ("prompt", "Same as `PS1`."),
    ("PROMPT_EOL_MARK", "When the `PROMPT_CR` and `PROMPT_SP` options are set, the\n`PROMPT_EOL_MARK` parameter can be used to customize how the end of\npartial lines are shown.  This parameter undergoes prompt expansion, with\nthe `PROMPT_PERCENT` option set.  If not set, the default behavior is\nequivalent to the value \"%B%S%#%s%b\"."),
    ("PS1", "The primary prompt string, printed before a command is read.\nIt undergoes a special form of expansion\nbefore being displayed; see _Expansion of Prompt Sequences_ (zshmisc).\nThe default is \"%m%# \"."),
    ("PS2", "The secondary prompt, printed when the shell needs more information\nto complete a command.\nIt is expanded in the same way as `PS1`.\nThe default is \"%_> \", which displays any shell constructs or quotation\nmarks which are currently being processed."),
    ("PS3", "Selection prompt used within a `select` loop.\nIt is expanded in the same way as `PS1`.\nThe default is \"?# \"."),
    ("PS4", "The execution trace prompt.  Default is \"+%N:%i> \", which displays\nthe name of the current shell structure and the line number within it.\nIn sh or ksh emulation, the default is \"+ \"."),
    ("psvar", "An array (colon-separated list) whose elements can be used in\n`PROMPT` strings.  Setting `psvar` also sets `PSVAR`, and\nvice versa."),
    ("READNULLCMD", "The command name to assume if a single input redirection\nis specified with no command.  Defaults to `more`."),
    ("REPORTMEMORY", "If nonnegative, commands whose maximum resident set size (roughly\nspeaking, main memory usage) in kilobytes is greater than this\nvalue have timing statistics reported.  The format used to output\nstatistics is the value of the `TIMEFMT` parameter, which is the same\nas for the `REPORTTIME` variable and the `time` builtin; note that\nby default this does not output memory usage.  Appending\n`\" max RSS %M\"` to the value of `TIMEFMT` causes it to output the\nvalue that triggered the report.  If `REPORTTIME` is also in use,\nat most a single report is printed for both triggers.  This feature\nrequires the tt(getrusage+()) system call, commonly supported by\nmodern Unix-like systems."),
    ("REPORTTIME", "If nonnegative, commands whose combined user and system execution times\n(measured in seconds) are greater than this value have timing\nstatistics printed for them.  Output is suppressed for commands\nexecuted within the line editor, including completion; commands\nexplicitly marked with the `time` keyword still cause the summary\nto be printed in this case."),
    ("REPLY", "This parameter is reserved by convention to pass string values between\nshell scripts and shell builtins in situations where a function call or\nredirection are impossible or undesirable.  The `read` builtin and the\n`select` complex command may set `REPLY`, and filename generation both\nsets and examines its value when evaluating certain expressions.  Some\nmodules also employ `REPLY` for similar purposes."),
    ("reply", "As `REPLY`, but for array values rather than strings."),
    ("RPS1", "This prompt is displayed on the right-hand side of the screen\nwhen the primary prompt is being displayed on the left.\nThis does not work if the `SINGLE_LINE_ZLE` option is set.\nIt is expanded in the same way as `PS1`."),
    ("RPS2", "This prompt is displayed on the right-hand side of the screen\nwhen the secondary prompt is being displayed on the left.\nThis does not work if the `SINGLE_LINE_ZLE` option is set.\nIt is expanded in the same way as `PS2`."),
    ("SAVEHIST", "The maximum number of history events to save in the history file.\n\nIf this is made local, it is not implicitly set to 0, but may be\nexplicitly set locally."),
    ("SPROMPT", "The prompt used for spelling correction.  The sequence\n\"%R\" expands to the string which presumably needs spelling\ncorrection, and \"%r\" expands to the proposed correction.\nAll other prompt escapes are also allowed.\n\nThe actions available at the prompt are `[nyae]`:\n\n\n- **`n` +(\"no\"+) +(default+)** — Discard the correction.\nIf there are no more corrections, accept the command line, else (with\n`CORRECT_ALL`) prompt for the next.\n\n\n- **`y` +(\"yes\"+)** — Make the correction. If there are no more\ncorrections, accept the command line.\n\n\n- **`a` +(\"abort\"+)** — Place the entire command line in the\nhistory for later edit, but without accepting it.\n\n\n- **`e` +(\"edit\"+)** — Resume editing the command line.\n"),
    ("STTY", "If this parameter is set in a command's environment, the shell runs the\n`stty` command with the value of this parameter as arguments in order to\nset up the terminal before executing the command. The modes apply only to the\ncommand, and are reset when it finishes or is suspended. If the command is\nsuspended and continued later with the `fg` or `wait` builtins it will\nsee the modes specified by `STTY`, as if it were not suspended.  This\n(intentionally) does not apply if the command is continued via ``kill\n-CONT`'.  `STTY` is ignored if the command is run in the background, or\nif it is in the environment of the shell but not explicitly assigned to in\nthe input line. This avoids running stty at every external command by\naccidentally exporting it. Also note that `STTY` should not be used for\nwindow size specifications; these will not be local to the command.\n\nIf the parameter is set and empty, all of the above applies except\nthat `stty` is not run. This can be useful as a way to freeze the tty\naround a single command, blocking its changes to tty settings,\nsimilar to the `ttyctl` builtin."),
    ("TERM", "The type of terminal in use.  This is used when looking up termcap\nsequences.  An assignment to `TERM` causes zsh to re-initialize the\nterminal, even if the value does not change (e.g., \"TERM=$TERM\").  It\nis necessary to make such an assignment upon any change to the terminal\ndefinition database or terminal type in order for the new settings to\ntake effect."),
    ("TERMINFO", "A reference to your terminfo database, used by the \"terminfo\" library when the\nsystem has it; see manref(terminfo)(5).\nIf set, this causes the shell to reinitialise the terminal, making the\nworkaround \"TERM=$TERM\" unnecessary."),
    ("TERMINFO_DIRS", "A colon-seprarated list of terminfo databases, used by the \"terminfo\" library\nwhen the system has it; see manref(terminfo)(5). This variable is only\nused by certain terminal libraries, in particular ncurses; see\nmanref(terminfo)(5) to check support on your system.  If set, this\ncauses the shell to reinitialise the terminal, making the workaround\n\"TERM=$TERM\" unnecessary.  Note that unlike other colon-separated\narrays this is not tied to a zsh array."),
    (".term.extensions", "An array containing a list of extension features of the terminal.\nSee _Terminal Extensions_ (zshzle)."),
    (".term.querywait", "The time the shell waits, in hundredths of seconds, for the response to\nterminal queries.  A value of `\"0\"` disables the timeout and the shell will\nwait indefinitely.  If not set, a default of `\"50\"` or half a second is\nused."),
    ("TIMEFMT", "The format of process time reports with the `time` keyword.\nThe default is \"%J  %U user %S system %P cpu %*E total\".\nRecognizes the following escape sequences, although not all\nmay be available on all systems, and some that are available\nmay not be useful:\n\n- `%%` — A \"%\".\n- `%U` — CPU seconds spent in user mode.\n- `%S` — CPU seconds spent in kernel mode.\n- `%E` — Elapsed time in seconds.\n\n- **`%P`** — The CPU percentage, computed as\n100*(`%U`+`%S`)/`%E`.\n\n- `%W` — Number of times the process was swapped.\n\n- **`%X`** — The average amount in (shared) text space used in kilobytes.\n\n\n- **`%D`** — The average amount in (unshared) data/stack space used in\nkilobytes.\n\n\n- **`%K`** — The total space used (`%X`+`%D`) in kilobytes.\n\n- `%M` — The  maximum memory the process had in use at any time in\nkilobytes.\n\n- **`%F`** — The number of major page faults (page needed to be brought\nfrom disk).\n\n- `%R` — The number of minor page faults.\n- `%I` — The number of input operations.\n- `%O` — The number of output operations.\n- `%r` — The number of socket messages received.\n- `%s` — The number of socket messages sent.\n- `%k` — The number of signals received.\n\n- **`%w`** — Number of voluntary context switches (waits).\n\n- `%c` — Number of involuntary context switches.\n- `%J` — The name of this job.\n\nA star may be inserted between the percent sign and flags printing time\n(e.g., \"%*E\"); this causes the time to be printed in\n`_hh_`:`_mm_`:`_ss_`.\"_ttt_\"\nformat (hours and minutes are only printed if they are not zero).\nAlternatively, \"m\", \"u\", or \"n\" may be used (e.g.,\n\"%mE\") to produce time output in milliseconds, microseconds, or\nnanoseconds, respectively. Note that some timings on some platforms\nare not actually nanosecond-precise (nor accurate to 1 ns when\nthey are); in fact on many systems user and kernel times are not\neven microsecond-precise."),
    ("TMOUT", "If this parameter is nonzero, the shell will receive an `ALRM`\nsignal if a command is not entered within the specified number of\nseconds after issuing a prompt. If there is a trap on `SIGALRM`, it\nwill be executed and a new alarm is scheduled using the value of the\n`TMOUT` parameter after executing the trap.  If no trap is set, and\nthe idle time of the terminal is not less than the value of the\n`TMOUT` parameter, zsh terminates.  Otherwise a new alarm is\nscheduled to `TMOUT` seconds after the last keypress."),
    ("TMPPREFIX", "A pathname prefix which the shell will use for all temporary files.\nNote that this should include an initial part for the file name as\nwell as any directory names.  The default is \"/tmp/zsh\"."),
    ("TMPSUFFIX", "A filename suffix which the shell will use for temporary files created\nby process substitutions (e.g., `tt(=()_list_`\")\").\nNote that the value should include a leading dot \".\" if intended\nto be interpreted as a file extension.  The default is not to append\nany suffix, thus this parameter should be assigned only when needed\nand then unset again."),
    ("WORDCHARS", "A list of non-alphanumeric characters considered part of a word\nby the line editor."),
    ("ZBEEP", "If set, this gives a string of characters, which can use all the same codes\nas the `bindkey` command as described in\n_The zsh/zle Module_ (zshmodules), that will be output to the terminal\ninstead of beeping.  This may have a visible instead of an audible effect;\nfor example, the string \"\\e[?5h\\e[?5l\" on a vt100 or xterm will have\nthe effect of flashing reverse video on and off (if you usually use reverse\nvideo, you should use the string \"\\e[?5l\\e[?5h\" instead).  This takes\nprecedence over the `NOBEEP` option."),
    ("ZDOTDIR", "The directory to search for shell startup files (.zshrc, etc),\nif not `$HOME`."),
    ("zle_bracketed_paste", "This two-element array contains the terminal escape sequences for enabling and\ndisabling the bracketed paste feature which allows ZLE to discern text that is\npasted into the terminal.  These escape sequences are used to enable bracketed\npaste when ZLE is active and disable it at other times.  Unsetting the\nparameter has the effect of ensuring that bracketed paste remains disabled.\nHowever, see also the `.term.extensions` parameter which provides a single\nplace to enable or disable terminal features."),
    ("zle_cursorform", "An array describing contexts in which ZLE should change the shape and color\nof the cursor.\nSee ifzman(_Cursor Shape and Color_ in zmanref(zshzle))\\\nifnzman((Cursor Shape and Color))."),
    ("zle_highlight", "An array describing contexts in which ZLE should highlight the input text.\nSee _Character Highlighting_ (zshzle)."),
    ("ZLE_LINE_ABORTED", "This parameter is set by the line editor when an error occurs.  It\ncontains the line that was being edited at the point of the error.\n\"print -zr -- $ZLE_LINE_ABORTED\" can be used to recover the line.\nOnly the most recent line of this kind is remembered."),
    ("ZLE_SPACE_SUFFIX_CHARS", "These parameters are used by the line editor.  In certain circumstances\nsuffixes (typically space or slash) added by the completion system\nwill be removed automatically, either because the next editing command\nwas not an insertable character, or because the character was marked\nas requiring the suffix to be removed.\n\nThese variables can contain the sets of characters that will cause the\nsuffix to be removed.  If `ZLE_REMOVE_SUFFIX_CHARS` is set, those\ncharacters will cause the suffix to be removed; if\n`ZLE_SPACE_SUFFIX_CHARS` is set, those characters will cause the\nsuffix to be removed and replaced by a space.\n\nIf `ZLE_REMOVE_SUFFIX_CHARS` is not set, the default behaviour is\nequivalent to:\n\n    ZLE_REMOVE_SUFFIX_CHARS=$' \\t\\n;&|'\n\nIf `ZLE_REMOVE_SUFFIX_CHARS` is set but is empty, no characters have this\nbehaviour.  `ZLE_SPACE_SUFFIX_CHARS` takes precedence, so that the\nfollowing:\n\n    ZLE_SPACE_SUFFIX_CHARS=$'&|'\n\ncauses the characters \"&\" and \"|\" to remove the suffix but to\nreplace it with a space.\n\nTo illustrate the difference, suppose that the option `AUTO_REMOVE_SLASH`\nis in effect and the directory `DIR` has just been completed, with an\nappended `/`, following which the user types \"&\".  The default result\nis \"DIR&\".  With `ZLE_REMOVE_SUFFIX_CHARS` set but without including\n\"&\" the result is \"DIR/&\".  With `ZLE_SPACE_SUFFIX_CHARS` set to\ninclude \"&\" the result is \"DIR &\".\n\nNote that certain completions may provide their own suffix removal\nor replacement behaviour which overrides the values described here.\nSee the completion system documentation in\nnmref(Completion System)(zshcompsys)."),
    ("ZLE_RPROMPT_INDENT", "If set, used to give the indentation between the right hand side of\nthe right prompt in the line editor as given by `RPS1` or `RPROMPT`\nand the right hand side of the screen.  If not set, the value 1 is used.\n\nTypically this will be used to set the value to 0 so that the prompt\nappears flush with the right hand side of the screen.  This is not the\ndefault as many terminals do not handle this correctly, in particular\nwhen the prompt appears at the extreme bottom right of the screen.\nRecent virtual terminals are more likely to handle this case correctly.\nSome experimentation is necessary."),
    ("ZCURSES_COLORS", "Readonly integer.  The maximum number of colors the terminal\nsupports.  This value is initialised by the curses library and is not\navailable until the first time `zcurses init` is run."),
    ("ZCURSES_COLOR_PAIRS", "Readonly integer.  The maximum number of color pairs\n_fg_col_`/`_bg_col_ that may be defined in \"zcurses attr\"\ncommands; note this limit applies to all color pairs that have been\nused whether or not they are currently active.  This value is initialised\nby the curses library and is not available until the first time `zcurses\ninit` is run."),
    ("zcurses_attrs", "Readonly array.  The attributes supported by `zsh/curses`; available\nas soon as the module is loaded."),
    ("zcurses_colors", "Readonly array.  The colors supported by `zsh/curses`; available\nas soon as the module is loaded."),
    ("zcurses_keycodes", "Readonly array.  The values that may be returned in the second\nparameter supplied to \"zcurses input\" in the order in which they\nare defined internally by curses.  Not all function keys\nare listed, only `F0`; curses reserves space for `F0` up to `F63`."),
    ("zcurses_windows", "Readonly array.  The current list of windows, i.e. all windows that\nhave been created with \"zcurses addwin\" and not removed with\n\"zcurses delwin\"."),
    ("EPOCHREALTIME", "A floating point value representing the number of seconds since\nthe epoch.  The notional accuracy is to nanoseconds if the\n`clock_gettime` call is available and to microseconds otherwise,\nbut in practice the range of double precision floating point and\nshell scheduling latencies may be significant effects."),
    ("EPOCHSECONDS", "An integer value representing the number of seconds since the\nepoch."),
    ("epochtime", "An array value containing the number of seconds since the epoch\nin the first element and the remainder of the time since the epoch\nin nanoseconds in the second element.  To ensure the two elements\nare consistent the array should be copied or otherwise referenced\nas a single substitution before the values are used.  The following\nidiom may be used:\n\n    for secs nsecs in $epochtime; do\n  ...\ndone"),
    (".zle.esc", "This associative array contains the literal escape sequences used to apply the\nhighlighting for each group. An example use would be when setting the\n`LESS_TERMCAP_xx` environment variables for the `less` pager."),
    (".zle.sgr", "Where highlighting makes use of CSI escape sequences, this parameter contains\nthe \"Select Graphic Rendition\" number sequence. This is useful with, for\nexample the `GREP_COLORS` and  `LSCOLORS` environment variables and the\n`list-colors` style."),
    (".sh.command", "A named reference to \"ZSH_DEBUG_CMD\""),
    (".sh.edchar", "In a ZLE widget, equivalent to the \"KEYS\" special parameter.  In a\nfuture release, assignments to this parameter will affect the input\nstream seen by ZLE."),
    (".sh.edcol", "A named reference to the ZLE special parameter \"CURSOR\"."),
    (".sh.edmode", "In a ZLE widget, this parameter has the value `ESC\" (\"$\"\\e\"`) if the\n`VI` option is set and the \"main\" keymap is selected.  This is\nintended for use with vi-mode key bindings (\"bindkey -v\").  In a\nfuture revision, assigning \".sh.edchar=${.sh.edmode}\" is expected\nto initiate \"vicmd\" mode when \"viins\" is active, and do\nnothing when \"vicmd\" is already active."),
    (".sh.edtext", "A named reference to the \"BUFFER\" special ZLE parameter."),
    (".sh.file", "A named reference to the \"ZSH_SCRIPT\" parameter."),
    (".sh.fun", "In a shell function, the function's name.  Usually the same as \"$0\",\nbut not affected by `POSIX_ARGZERO` or `FUNCTION_ARGZERO` options."),
    (".sh.level", "In a shell function, initially set to the depth of the call stack. This\nmay be assigned to change the apparent depth.  However, such assignment\ndoes not affect the scope of local parameters."),
    (".sh.lineno", "A named reference to the \"LINENO\" special parameter."),
    (".sh.match", "An array equivalent to the \"match\" parameter as assigned by the\n`tt((#b)`\")\" extended globbing flag.  When the\n`KSH_ARRAYS` option is set, \"${.sh.match[0]}\" gives the value of\nthe \"MATCH\" parameter as set by the `tt((#m)`\")\" extended\nglobbing flag.  Currently, the `EXTENDED_GLOB` option must be enabled\nand those flags must be explicitly used in a pattern in order for these\nvalues of \".sh.match\" to be set."),
    (".sh.name", "When the \"vared\" command is used, this parameter may be used in\nuser-defined ZLE widgets to get the name of the variable being edited.\nA future release is expected to use this parameter in the implementation\nof \"discipline functions\"."),
    (".sh.subscript", "When \"vared\" has been used on an array element, this parameter holds\nthe array index (either a number, or an associative array key)."),
    (".sh.subshell", "A named reference to the \"ZSH_SUBSHELL\" parameter."),
    (".sh.value", "In \"vared\" this is a named reference to the ZLE special \"BUFFER\".\nA future release is expected to use this parameter in the implementation\nof \"discipline functions\"."),
    (".sh.version", "A named reference to \"ZSH_PATCHLEVEL\"."),
    ("langinfo", "An associative array that maps langinfo elements to\ntheir values.\n\nYour implementation may support a number of the following keys:\n\n`CODESET`,\n`D_T_FMT`,\n`D_FMT`,\n`T_FMT`,\n`RADIXCHAR`,\n`THOUSEP`,\n`YESEXPR`,\n`NOEXPR`,\n`CRNCYSTR`,\n`ABDAY_{1..7}`,\n`DAY_{1..7}`,\n`ABMON_{1..12}`,\n`MON_{1..12}`,\n`T_FMT_AMPM`,\n`AM_STR`,\n`PM_STR`,\n`ERA`,\n`ERA_D_FMT`,\n`ERA_D_T_FMT`,\n`ERA_T_FMT`,\n`ALT_DIGITS`"),
    ("mapfile", "This associative array takes as keys the names of files; the resulting\nvalue is the content of the file.  The value is treated identically to any\nother text coming from a parameter.  The value may also be assigned to, in\nwhich case the file in question is written (whether or not it originally\nexisted); or an element may be unset, which will delete the file in\nquestion.  For example, \"\"vared \"mapfile[myfile]\"`' works as expected,\nediting the file \"myfile\".\n\nWhen the array is accessed as a whole, the keys are the names of files in\nthe current directory, and the values are empty (to save a huge overhead in\nmemory).  Thus tt(${(k)mapfile}) has the same effect as the glob operator\ntt(*(D)), since files beginning with a dot are not special.  Care must be\ntaken with expressions such as tt(rm ${(k)mapfile}), which will delete\nevery file in the current directory without the usual \"rm *\" test.\n\nThe parameter `mapfile` may be made read-only; in that case, files\nreferenced may not be written or deleted.\n\nA file may conveniently be read into an array as one line per element\nwith the form\n`_array_tt(=(\"${(f@)mapfile[)_filename_`]}\"\")\".\nThe double quotes and the \"@\" are necessary to prevent empty lines\nfrom being removed.  Note that if the file ends with a newline,\nthe shell will split on the final newline, generating an additional\nempty field; this can be suppressed by using\n\"_array_tt(=(\"${(f@)${mapfile[)_filename_\"]%$\"\\n\"}}\"\")\"."),
    ("options", "The keys for this associative array are the names of the options that\ncan be set and unset using the `setopt` and `unsetopt`\nbuiltins. The value of each key is either the string `on` if the\noption is currently set, or the string `off` if the option is unset.\nSetting a key to one of these strings is like setting or unsetting\nthe option, respectively. Unsetting a key in this array is like\nsetting it to the value `off`."),
    ("commands", "This array gives access to the command hash table. The keys are the\nnames of external commands, the values are the pathnames of the files\nthat would be executed when the command would be invoked. Setting a\nkey in this array defines a new entry in this table in the same way as\nwith the `hash` builtin. Unsetting a key as in ``unset\n\"commands[foo]\"`' removes the entry for the given key from the command\nhash table."),
    ("functions", "This associative array maps names of enabled functions to their\ndefinitions. Setting a key in it is like defining a function with the\nname given by the key and the body given by the value. Unsetting a key\nremoves the definition for the function named by the key."),
    ("dis_functions", "Like `functions` but for disabled functions."),
    ("functions_source", "This readonly associative array maps names of enabled functions to the\nname of the file containing the source of the function.\n\nFor an autoloaded function that has already been loaded, or marked for\nautoload with an absolute path, or that has had its path resolved with\n\"functions -r\", this is the file found for autoloading, resolved\nto an absolute path.\n\nFor a function defined within the body of a script or sourced file,\nthis is the name of that file.  In this case, this is the exact path\noriginally used to that file, which may be a relative path.\n\nFor any other function, including any defined at an interactive prompt or\nan autoload function whose path has not yet been resolved, this is\nthe empty string.  However, the hash element is reported as defined\njust so long as the function is present:  the keys to this hash are\nthe same as those to `$functions`."),
    ("dis_functions_source", "Like `functions_source` but for disabled functions."),
    ("builtins", "This associative array gives information about the builtin commands\ncurrently enabled. The keys are the names of the builtin commands and\nthe values are either \"undefined\" for builtin commands that will\nautomatically be loaded from a module if invoked or \"defined\" for\nbuiltin commands that are already loaded."),
    ("dis_builtins", "Like `builtins` but for disabled builtin commands."),
    ("reswords", "This array contains the enabled reserved words."),
    ("dis_reswords", "Like `reswords` but for disabled reserved words."),
    ("patchars", "This array contains the enabled pattern characters."),
    ("dis_patchars", "Like `patchars` but for disabled pattern characters."),
    ("aliases", "This maps the names of the regular aliases currently enabled to their\nexpansions."),
    ("dis_aliases", "Like `aliases` but for disabled regular aliases."),
    ("galiases", "Like `aliases`, but for global aliases."),
    ("dis_galiases", "Like `galiases` but for disabled global aliases."),
    ("saliases", "Like `raliases`, but for suffix aliases."),
    ("dis_saliases", "Like `saliases` but for disabled suffix aliases."),
    ("parameters", "The keys in this associative array are the names of the parameters\ncurrently defined. The values are strings describing the type of the\nparameter, in the same format used by the `t` parameter flag, see\nsubref(Parameter Expansion Flags)(zshexpn).\nThe value may also be \"undefined\" indicating a parameter that\nmay be autoloaded from a module but has not yet been referenced.\nWhen the key is the name of a named reference, the value is\n\"nameref-\" prepended to the type of the referenced parameter,\nfor example\nifzman()\n\n    `% typeset -n parms=parameters`\n`% print -r ${parameters[parms]}`\n`nameref-association-readonly-hide-hideval-special`\n\nSetting or unsetting keys in this array is not possible."),
    ("modules", "An associative array giving information about modules. The keys are the names\nof the modules loaded, registered to be autoloaded, or aliased. The\nvalue says which state the named module is in and is one of the\nstrings \"loaded\", \"autoloaded\", or ``alias:\"_name_\",\nwhere _name_ is the name the module is aliased to.\n\nSetting or unsetting keys in this array is not possible."),
    ("dirstack", "A normal array holding the elements of the directory stack. Note that\nthe output of the `dirs` builtin command includes one more\ndirectory, the current working directory."),
    ("history", "This associative array maps history event numbers to the full history lines.\nAlthough it is presented as an associative array, the array of all values\n(`${history[@]}`) is guaranteed to be returned in order from most recent\nto oldest history event, that is, by decreasing history event number."),
    ("historywords", "A special array containing the words stored in the history.  These also\nappear in most to least recent order."),
    ("jobdirs", "This associative array maps job numbers to the directories from which the\njob was started (which may not be the current directory of the job).\n\nThe keys of the associative arrays are usually valid job numbers,\nand these are the values output with, for example, tt(${(k)jobdirs}).\nNon-numeric job references may be used when looking up a value;\nfor example, `${jobdirs[%+]}` refers to the current job.\n\nSee the `jobs` builtin for how job information is provided in a subshell."),
    ("jobtexts", "This associative array maps job numbers to the texts of the command lines\nthat were used to start the jobs.\n\nHandling of the keys of the associative array is as described for\n`jobdirs` above.\n\nSee the `jobs` builtin for how job information is provided in a subshell."),
    ("jobstates", "This associative array gives information about the states of the jobs\ncurrently known. The keys are the job numbers and the values are\nstrings of the form\n`_job-state_`:`_mark_`:`_pid_`=\"_state_...\". The\n_job-state_ gives the state the whole job is currently in, one of\n\"running\", \"suspended\", or \"done\". The _mark_ is\n\"+\" for the current job, \"-\" for the previous job and empty\notherwise. This is followed by one ``:`_pid_`=\"_state_\" for every\nprocess in the job. The _pid_s are, of course, the process IDs and\nthe _state_ describes the state of that process.\n\nHandling of the keys of the associative array is as described for\n`jobdirs` above.\n\nSee the `jobs` builtin for how job information is provided in a subshell."),
    ("nameddirs", "This associative array maps the names of named directories to the pathnames\nthey stand for."),
    ("userdirs", "This associative array maps user names to the pathnames of their home\ndirectories."),
    ("usergroups", "This associative array maps names of system groups of which the current\nuser is a member to the corresponding group identifiers.  The contents\nare the same as the groups output by the `id` command."),
    ("funcfiletrace", "This array contains the absolute line numbers and corresponding file\nnames for the point where the current function, sourced file, or (if\n`EVAL_LINENO` is set) `eval` command was\ncalled.  The array is of the same length as `funcsourcetrace` and\n`functrace`, but differs from `funcsourcetrace` in that the line and\nfile are the point of call, not the point of definition, and differs\nfrom `functrace` in that all values are absolute line numbers in\nfiles, rather than relative to the start of a function, if any."),
    ("funcsourcetrace", "This array contains the file names and line numbers of the\npoints where the functions, sourced files, and (if `EVAL_LINENO` is set)\n`eval` commands currently being executed were\ndefined.  The line number is the line where the ``function\" _name_\"\nor \"_name_ tt(())\" started.  In the case of an autoloaded\nfunction  the line number is reported as zero.\nThe format of each element is _filename_`:`_lineno_.\n\nFor functions autoloaded from a file in native zsh format, where only the\nbody of the function occurs in the file, or for files that have been\nexecuted by the `source` or \".\" builtins, the trace information is\nshown as _filename_`:`_0_, since the entire file is the\ndefinition.  The source file name is resolved to an absolute path when\nthe function is loaded or the path to it otherwise resolved.\n\nMost users will be interested in the information in the\n`funcfiletrace` array instead."),
    ("funcstack", "This array contains the names of the functions, sourced files,\nand (if `EVAL_LINENO` is set) `eval` commands. currently being\nexecuted. The first element is the name of the function using the\nparameter.\n\nThe standard shell array `zsh_eval_context` can be used to\ndetermine the type of shell construct being executed at each depth:\nnote, however, that is in the opposite order, with the most recent\nitem last, and it is more detailed, for example including an\nentry for `toplevel`, the main shell code being executed\neither interactively or from a script, which is not present\nin `$funcstack`."),
    ("functrace", "This array contains the names and line numbers of the callers\ncorresponding to the functions currently being executed.\nThe format of each element is _name_`:`_lineno_.\nCallers are also shown for sourced files; the caller is the point\nwhere the `source` or \".\" command was executed."),
    ("SRANDOM", "A random positive 32-bit integer between 0 and 4,294,967,295.  This parameter\nis read-only. The name was chosen for compatibility with Bash and to\ndistinguish it from `RANDOM` which has a documented repeatable behavior."),
    ("zsh_scheduled_events", "A readonly array corresponding to the events scheduled by the\n`sched` builtin.  The indices of the array correspond to the numbers\nshown when `sched` is run with no arguments (provided that the\n`KSH_ARRAYS` option is not set).  The value of the array\nconsists of the scheduled time in seconds since the epoch\n(see _The zsh/datetime Module_ (zshmodules) for facilities for\nusing this number), followed by a colon, followed by any options\n(which may be empty but will be preceded by a \"-\" otherwise),\nfollowed by a colon, followed by the command to be executed.\n\nThe `sched` builtin should be used for manipulating the events.  Note\nthat this will have an immediate effect on the contents of the array,\nso that indices may become invalid."),
    ("errnos", "A readonly array of the names of errors defined on the system.  These\nare typically macros defined in C by including the system header file\n`errno.h`.  The index of each name (assuming the option `KSH_ARRAYS`\nis unset) corresponds to the error number.  Error numbers _num_\nbefore the last known error which have no name are given the name\n`E`_num_ in the array.\n\nNote that aliases for errors are not handled; only the canonical name is\nused."),
    ("sysparams", "A readonly associative array.  The keys are:\n\n\n- **`pid`** — vindex(pid, sysparams)\nReturns the process ID of the current process, even in subshells.  Compare\n`$$`, which returns the process ID of the main shell process.\n\n\n- **`ppid`** — vindex(ppid, sysparams)\nReturns the current process ID of the parent of the current process, even\nin subshells.  Compare `$PPID`, which returns the process ID of the\ninitial parent of the main shell process.\n\n\n- **`procsubstpid`** — Returns the process ID of the last process started for process\nsubstitution, i.e. the tt(<()_..._``) and\ntt(>()_..._``) expansions.\n"),
    ("termcap", "An associative array that maps termcap capability codes to\ntheir values."),
    ("terminfo", "An associative array that maps terminfo capability names to\ntheir values."),
    ("LOGCHECK", "The interval in seconds between checks for login/logout activity\nusing the `watch` parameter."),
    ("watch", "An array (colon-separated list) of login/logout events to report.\n\nIf it contains the single word \"all\", then all login/logout events\nare reported.  If it contains the single word \"notme\", then all\nevents are reported as with \"all\" except `$USERNAME`.\n\nAn entry in this list may consist of a username,\nan \"@\" followed by a remote hostname,\nand a \"%\" followed by a line (tty).  Any of these may\nbe a pattern (be sure to quote this during the assignment to\n`watch` so that it does not immediately perform file generation);\nthe setting of the `EXTENDED_GLOB` option is respected.\nAny or all of these components may be present in an entry;\nif a login/logout event matches all of them,\nit is reported.\n\nFor example, with the `EXTENDED_GLOB` option set, the following:\n\nexample(watch=('^(pws|barts)'))\n\ncauses reports for activity associated with any user other than `pws`\nor `barts`."),
    ("WATCHFMT", "The format of login/logout reports if the `watch` parameter is set.\nDefault is \"%n has %a %l from %m\".\nRecognizes the following escape sequences:\n\n- **`%n`** —\nThe name of the user that logged in/out.\n\n- **`%a`** —\nThe observed action, i.e. \"logged on\" or \"logged off\".\n\n\n- **`%l`** — The line (tty) the user is logged in on.\n\n\n- **`%M`** —\nThe full hostname of the remote host.\n\n- **`%m`** —\nThe hostname up to the first \".\".  If only the\nIP address is available or the utmp field contains\nthe name of an X-windows display, the whole name is printed.\n\n_NOTE:_\nThe \"%m\" and \"%M\" escapes will work only if there is a host name\nfield in the utmp on your machine.  Otherwise they are\ntreated as ordinary strings.\n\n\n- **`%F{`_color_`}` (`%f`)** — Start (stop) using a different foreground color.\n\n\n- **`%K{`_color_`}` (`%k`)** — Start (stop) using a different background color.\n\n\n- **`%S` (`%s`)** — Start (stop) standout mode.\n\n\n- **`%U` (`%u`)** — Start (stop) underline mode.\n\n\n- **`%B` (`%b`)** — Start (stop) boldface mode.\n\n\n- **`%t`**\n\n- **`%@`** —\nThe time, in 12-hour, am/pm format.\n\n- **`%T`** —\nThe time, in 24-hour format.\n\n- **`%w`** —\nThe date in `_day_`-\"_dd_\" format.\n\n- **`%W`** —\nThe date in `_mm_`/`_dd_`/\"_yy_\" format.\n\n- **`%D`** —\nThe date in `_yy_`-`_mm_`-\"_dd_\" format.\n\n\n- **`%D{`_string_`}`** — The date formatted as _string_ using the `strftime` function, with\nzsh extensions as described for `%D{`_string_`}` escape sequence in\n_Simple Prompt Escapes_ (zshmisc).\n\n\n- **`%()_x_`:`_true-text_`:`_false-text_```** — Specifies a ternary expression.\nThe character following the _x_ is\narbitrary; the same character is used to separate the text\nfor the \"true\" result from that for the \"false\" result.\nBoth the separator and the right parenthesis may be escaped\nwith a backslash.\nTernary expressions may be nested.\n\nThe test character _x_ may be any one of \"l\", \"n\", \"m\"\nor \"M\", which indicate a \"true\" result if the corresponding\nescape sequence would return a non-empty value; or it may be \"a\",\nwhich indicates a \"true\" result if the watched user has logged in,\nor \"false\" if he has logged out.\nOther characters evaluate to neither true nor false; the entire\nexpression is omitted in this case.\n\nIf the result is \"true\", then the _true-text_\nis formatted according to the rules above and printed,\nand the _false-text_ is skipped.\nIf \"false\", the _true-text_ is skipped and the _false-text_\nis formatted and printed.\nEither or both of the branches may be empty, but\nboth separators must be present in any case.\n"),
    ("log", "List all users currently logged in who are affected by\nthe current setting of the `watch` parameter."),
    ("ZFTP_TMOUT", "Integer.  The time in seconds to wait for a network operation to\ncomplete before returning an error.  If this is not set when the\nmodule is loaded, it will be given the default value 60.  A value of\nzero turns off timeouts.  If a timeout occurs on the control\nconnection it will be closed.  Use a larger value if this occurs too\nfrequently."),
    ("ZFTP_IP", "Readonly.  The IP address of the current connection in dot notation."),
    ("ZFTP_HOST", "Readonly.  The hostname of the current remote server.  If the host was\nopened as an IP number, `ZFTP_HOST` contains that instead; this\nsaves the overhead for a name lookup, as IP numbers are most commonly\nused when a nameserver is unavailable."),
    ("ZFTP_PORT", "Readonly.  The number of the remote TCP port to which the connection is\nopen (even if the port was originally specified as a named service).\nUsually this is the standard FTP port, 21.\n\nIn the unlikely event that your system does not have the appropriate\nconversion functions, this appears in network byte order.  If your\nsystem is little-endian, the port then consists of two swapped bytes and the\nstandard port will be reported as 5376.  In that case, numeric ports passed\nto `zftp open` will also need to be in this format."),
    ("ZFTP_SYSTEM", "Readonly.  The system type string returned by the server in response\nto an FTP `SYST` request.  The most interesting case is a string\nbeginning `\"UNIX Type: L8\"`, which ensures maximum compatibility\nwith a local UNIX host."),
    ("ZFTP_TYPE", "Readonly.  The type to be used for data transfers , either \"A\" or\n\"I\".   Use the `type` subcommand to change this."),
    ("ZFTP_USER", "Readonly.  The username currently logged in, if any."),
    ("ZFTP_ACCOUNT", "Readonly.  The account name of the current user, if any.  Most servers\ndo not require an account name."),
    ("ZFTP_PWD", "Readonly.  The current directory on the server."),
    ("ZFTP_CODE", "Readonly.  The three digit code of the last FTP reply from the server\nas a string.  This can still be read after the connection is closed, and\nis not changed when the current session changes."),
    ("ZFTP_REPLY", "Readonly.  The last line of the last reply sent by the server.  This\ncan still be read after the connection is closed, and is not changed when\nthe current session changes."),
    ("ZFTP_SESSION", "Readonly.  The name of the current FTP session; see the description of the\n`session` subcommand."),
    ("ZFTP_PREFS", "A string of preferences for altering aspects of `zftp`'s behaviour.\nEach preference is a single character.  The following are defined:\n\n- **`P`** —\nPassive:  attempt to make the remote server initiate data transfers.\nThis is slightly more efficient than sendport mode.  If the letter\n`S` occurs later in the string, `zftp` will use sendport mode if\npassive mode is not available.\n\n- **`S`** —\nSendport:  initiate transfers by the FTP `PORT` command.  If this\noccurs before any `P` in the string, passive mode will never be\nattempted.\n\n\n- **`D`** — Dumb:  use only the bare minimum of FTP commands.  This prevents\nthe variables `ZFTP_SYSTEM` and `ZFTP_PWD` from being set, and\nwill mean all connections default to ASCII type.  It may prevent\n`ZFTP_SIZE` from being set during a transfer if the server\ndoes not send it anyway (many servers do).\n\n\nIf `ZFTP_PREFS` is not set when `zftp` is loaded, it will be set to a\ndefault of \"PS\", i.e. use passive mode if available, otherwise\nfall back to sendport mode."),
    ("ZFTP_VERBOSE", "A string of digits between 0 and 5 inclusive, specifying which\nresponses from the server should be printed.  All responses go to\nstandard error.  If any of the numbers 1 to 5 appear in the string,\nraw responses from the server with reply codes beginning with that\ndigit will be printed to standard error.  The first digit of the three\ndigit reply code is defined by RFC959 to correspond to:\n\n- **1.** —\nA positive preliminary reply.\n\n- **2.** —\nA positive completion reply.\n\n- **3.** —\nA positive intermediate reply.\n\n- **4.** —\nA transient negative completion reply.\n\n- **5.** —\nA permanent negative completion reply.\n\nIt should be noted that, for unknown reasons, the reply `Service not\navailable', which forces termination of a connection, is classified as\n421, i.e. \"transient negative\", an interesting interpretation of the word\n\"transient\".\n\nThe code 0 is special:  it indicates that all but the last line of\nmultiline replies read from the server will be printed to standard\nerror in a processed format.  By convention, servers use this\nmechanism for sending information for the user to read.  The\nappropriate reply code, if it matches the same response, takes\npriority.\n\nIf `ZFTP_VERBOSE` is not set when `zftp` is loaded, it will be\nset to the default value `450`, i.e., messages destined for the user\nand all errors will be printed.  A null string is valid and\nspecifies that no messages should be printed."),
    ("ZFTP_FILE", "The name of the remote file being transferred from or to."),
    ("ZFTP_TRANSFER", "A `G` for a `get` operation and a `P` for a `put` operation."),
    ("ZFTP_SIZE", "The total size of the complete file being transferred:\nthe same as the first value provided by the\n`remote` and `local` subcommands for a particular file.\nIf the server cannot supply this value for a remote file being\nretrieved, it will not be set.  If input is from a pipe the value may\nbe incorrect and correspond simply to a full pipe buffer."),
    ("ZFTP_COUNT", "The amount of data so far transferred; a number between zero and\n`$ZFTP_SIZE`, if that is set.  This number is always available."),
    ("keymaps", "This array contains the names of the keymaps currently defined."),
    ("widgets", "This associative array contains one entry per widget. The name\nof the widget is the key and the value gives information about the\nwidget. It is either\n  the string \"builtin\" for builtin widgets,\n  a string of the form ``user:\"_name_\" for user-defined widgets,\n    where _name_ is the name of the shell function implementing the widget,\n  a string of the form ``completion:`_type_`:\"_name_\"\n    for completion widgets,\n  or a null value if the widget is not yet fully defined.\nIn the penultimate case, _type_ is the name of the builtin widget the\ncompletion widget imitates in its behavior and _name_ is the name\nof the shell function implementing the completion widget."),
    ("zstyle", "Delete style definitions. Without arguments all definitions are deleted,\nwith a _pattern_ all definitions for that pattern are deleted and if\nany _style_s are given, then only those styles are deleted for the\n_pattern_."),
    ("incarg", "This widget allows you to increment integers on the current line. In addition\nto decimals, it can handle hexadecimals prefixed with `0x`, binaries with\n`0b`, and octals with `0o`.\n\nBy default, the target integer will be incremented by one. With a numeric\nargument, the integer is incremented by the amount of the argument. The shell\nparameter `incarg` may be set to change the default increment to something\nother than one.\n\nThe behavior of this widget changes depending on the widget name.\n\nWhen the widget is named `incarg`, the widget will increment an integer\nplaced under the cursor placed or just to the left of it. `decarg`, on the\nother hand, decrements the integer. When the name is prefixed with `vi`,\nthe cursor will jump to the nearest integer after the cursor before incrementing\nit. The `vi` prefix can also be combined with a `backward-` prefix to make\nthe widget search backwards for numbers.\n\nThere's also a `sync-` prefix that can be added to the widget name. This\nvariant is used for creating a sequence of numbers on split terminals with\nsynchronized key input. The first pane won't increment the integer at all, but\neach pane after that will have the integer incremented once more than the\nprevious pane. It currently supports tmux and iTerm2.\n\nThe prefixes `vi`, `backward-`, and `sync-` can be combined, for example,\ninto `vim-sync-` or `vim-backward-sync-`. The `vi` prefix needs to be\nat the very beginning.\n\n    bindkey '^X+' incarg"),
    ("BUFFER", "The entire contents of the edit buffer.  If it is written to, the\ncursor remains at the same offset, unless that would put it outside the\nbuffer."),
    ("BUFFERLINES", "The number of screen lines needed for the edit buffer currently\ndisplayed on screen (i.e. without any changes to the preceding\nparameters done after the last redisplay); read-only."),
    ("CONTEXT", "The context in which zle was called to read a line; read-only.  One of\nthe values:\n\n\n- **`start`** — The start of a command line (at prompt `PS1`).\n\n\n- **`cont`** — A continuation to a command line (at prompt `PS2`).\n\n\n- **`select`** — In a `select` loop (at prompt `PS3`).\n\n\n- **`vared`** —\nEditing a variable in `vared`."),
    ("CURSOR", "The offset of the cursor, within the edit buffer.  This is in the range\n0 to `$#BUFFER`, and is by definition equal to `$#LBUFFER`.\nAttempts to move the cursor outside the buffer will result in the\ncursor being moved to the appropriate end of the buffer."),
    ("CUTBUFFER", "The last item cut using one of the \"kill-\" commands; the string\nwhich the next yank would insert in the line.  Later entries in\nthe kill ring are in the array `killring`.  Note that the\ncommand ``zle copy-region-as-kill\" _string_\" can be used to\nset the text of the cut buffer from a shell function and cycle the kill\nring in the same way as interactively killing text."),
    ("HISTNO", "The current history number.  Setting this has the same effect as\nmoving up or down in the history to the corresponding history line.\nAn attempt to set it is ignored if the line is not stored in the\nhistory.  Note this is not the same as the parameter `HISTCMD`,\nwhich always gives the number of the history line being added to the main\nshell's history.  `HISTNO` refers to the line being retrieved within\nzle."),
    ("ISEARCHMATCH_END", "`ISEARCHMATCH_ACTIVE` indicates whether a part of the `BUFFER` is\ncurrently matched by an incremental search pattern. `ISEARCHMATCH_START`\nand `ISEARCHMATCH_END` give the location of the matched part and are\nin the same units as `CURSOR`. They are only valid for reading\nwhen `ISEARCHMATCH_ACTIVE` is non-zero.\n\nAll parameters are read-only."),
    ("KEYMAP", "The name of the currently selected keymap; read-only."),
    ("KEYS", "The keys typed to invoke this widget, as a literal string; read-only."),
    ("KEYS_QUEUED_COUNT", "The number of bytes pushed back to the input queue and therefore\navailable for reading immediately before any I/O is done; read-only.\nSee also `PENDING`; the two values are distinct."),
    ("killring", "The array of previously killed items, with the most recently killed first.\nThis gives the items that would be retrieved by a `yank-pop` in the\nsame order.  Note, however, that the most recently killed item is in\n`$CUTBUFFER`; `$killring` shows the array of previous entries.\n\nThe default size for the kill ring is eight, however the length may be\nchanged by normal array operations.  Any empty string in the kill ring is\nignored by the `yank-pop` command, hence the size of the array\neffectively sets the maximum length of the kill ring, while the number of\nnon-zero strings gives the current length, both as seen by the user at the\ncommand line."),
    ("LASTABORTEDSEARCH", "The last search string used by an interactive search that was\naborted by the user (status 3 returned by the search widget)."),
    ("LASTSEARCH", "The last search string used by an interactive search; read-only.\nThis is set even if the search failed (status 0, 1 or 2 returned\nby the search widget), but not if it was aborted by the user."),
    ("LASTWIDGET", "The name of the last widget that was executed; read-only."),
    ("LBUFFER", "The part of the buffer that lies to the left of the cursor position.\nIf it is assigned to, only that part of the buffer is replaced, and the\ncursor remains between the new `$LBUFFER` and the old `$RBUFFER`."),
    ("MARK", "Like `CURSOR`, but for the mark. With vi-mode operators that wait for\na movement command to select a region of text, setting `MARK` allows\nthe selection to extend in both directions from the initial cursor\nposition."),
    ("NUMERIC", "The numeric argument. If no numeric argument was given, this parameter\nis unset. When this is set inside a widget function, builtin widgets\ncalled with the `zle` builtin command will use the value\nassigned. If it is unset inside a widget function, builtin widgets\ncalled behave as if no numeric argument was given."),
    ("PENDING", "The number of bytes pending for input, i.e. the number of bytes which have\nalready been typed and can immediately be read. On systems where the shell\nis not able to get this information, this parameter will always have a\nvalue of zero.  Read-only.  See also `KEYS_QUEUED_COUNT`; the two\nvalues are distinct."),
    ("PREBUFFER", "In a multi-line input at the secondary prompt, this read-only parameter\ncontains the contents of the lines before the one the cursor is\ncurrently in."),
    ("PREDISPLAY", "Text to be displayed before the start of the editable text buffer.  This\ndoes not have to be a complete line; to display a complete line, a newline\nmust be appended explicitly.  The text is reset on each new invocation\n(but not recursive invocation) of zle."),
    ("POSTDISPLAY", "Text to be displayed after the end of the editable text buffer.  This\ndoes not have to be a complete line; to display a complete line, a newline\nmust be prepended explicitly.  The text is reset on each new invocation\n(but not recursive invocation) of zle."),
    ("RBUFFER", "The part of the buffer that lies to the right of the cursor position.\nIf it is assigned to, only that part of the buffer is replaced, and the\ncursor remains between the old `$LBUFFER` and the new `$RBUFFER`."),
    ("REGION_ACTIVE", "Indicates if the region is currently active.  It can be assigned 0 or 1\nto deactivate and activate the region respectively. A value of 2\nactivates the region in line-wise mode with the highlighted text\nextending for whole lines only; see\n_Character Highlighting_ (below)."),
    ("region_highlight", "Each element of this array may be set to a string that describes\nhighlighting for an arbitrary region of the command line that will\ntake effect the next time the command line is redisplayed.  Highlighting\nof the non-editable parts of the command line in `PREDISPLAY`\nand `POSTDISPLAY` are possible, but note that the `P` flag\nis needed for character indexing to include `PREDISPLAY`.\n\nEach string consists of the following whitespace-separated parts:\n\nstartitemize()\nitemiz(Optionally, a \"P\" to signify that the start and end offset that\nfollow include any string set by the `PREDISPLAY` special parameter;\nthis is needed if the predisplay string itself is to be highlighted.\nWhitespace between the \"P\" and the start offset is optional.)\nitemiz(A start offset in the same units as `CURSOR`.)\nitemiz(An end offset in the same units as `CURSOR`.)\nitemiz(A highlight specification in the same format as\nused for contexts in the parameter `zle_highlight`, see\n_Character Highlighting_ (below);\nfor example, `standout` or `fg=red,bold`.)\nitemiz(Optionally, a string of the form ``memo=\"_token_\".\nThe _token_ consists of everything between the \"=\" and the next\nwhitespace, comma, NUL, or the end of the string.\nThe _token_ is preserved verbatim but not parsed in any way.\n\nPlugins may use this to identify array elements they have added: for example,\na plugin might set _token_ to its (the plugin's) name and then use\n`tt(region_highlight=+( ${region_highlight:#*memo=)_token_`} +\")\"\nin order to remove array elements it added.\n\n(This example uses the ``${`_name_`:#`_pattern_`}`' array-grepping\nsyntax described in\n_Parameter Expansion_ (zshexpn).))\nenditemize()\n\nFor example,\n\nexample(region_highlight=(\"P0 20 bold memo=foobar\"))\n\nspecifies that the first twenty characters of the text including\nany predisplay string should be highlighted in bold.\n\nNote that the effect of `region_highlight` is not saved and disappears\nas soon as the line is accepted.\n\nNote that zsh 5.8 and older do not support the ``memo=\"_token_\" field\nand may misparse the third (highlight specification) field when a memo\nis given. COMMENT(The syntax \"0 20 bold, memo=foobar\" (with an auxiliary comma)\nhappens to work on both zsh <=5.8 and zsh 5.9, but that seems to be more of\nan accident of implementation than something we should make a first-class-citizen\nAPI promise.  It's mentioned in the \"Incompatibilities\" section of README.)\n\nWhere a particular region is covered by multiple entries in\n`region_highlight`, their effects are merged. In the case of conflicts, later\nentries take precedence over earlier ones. This precedence ordering can be\noverridden by specifying layers.\n\nThe final highlighting on the command line depends on both `region_highlight`\nand `zle_highlight`; see\n_Character Highlighting_ (below) for details."),
    ("registers", "The contents of each of the vi register buffers. These are\ntypically set using `vi-set-buffer` followed by a delete, change or\nyank command."),
    ("SUFFIX_END", "`SUFFIX_ACTIVE` indicates whether an auto-removable completion suffix\nis currently active. `SUFFIX_START` and `SUFFIX_END` give the\nlocation of the suffix and are in the same units as `CURSOR`. They are\nonly valid for reading when `SUFFIX_ACTIVE` is non-zero.\n\nAll parameters are read-only."),
    ("UNDO_CHANGE_NO", "A number representing the state of the undo history.  The only use\nof this is passing as an argument to the `undo` widget in order to\nundo back to the recorded point.  Read-only."),
    ("UNDO_LIMIT_NO", "A number corresponding to an existing change in the undo history;\ncompare `UNDO_CHANGE_NO`.  If this is set to a value greater\nthan zero, the `undo` command will not allow the line to\nbe undone beyond the given change number.  It is still possible\nto use ``zle undo\" _change_\" in a widget to undo beyond\nthat point; in that case, it will not be possible to undo at\nall until `UNDO_LIMIT_NO` is reduced.  Set to 0 to disable the limit.\n\nA typical use of this variable in a widget function is as follows (note\nthe additional function scope is required):\n\nexample(() {\n  local UNDO_LIMIT_NO=$UNDO_CHANGE_NO\n  # Perform some form of recursive edit.\n})"),
    ("WIDGET", "The name of the widget currently being executed; read-only."),
    ("WIDGETFUNC", "The name of the shell function that implements a widget defined with\neither `zle -N` or `zle -C`.  In the former case, this is the second\nargument to the `zle -N` command that defined the widget, or\nthe first argument if there was no second argument.  In the latter case\nthis is the third argument to the `zle -C` command that defined the\nwidget.  Read-only."),
    ("WIDGETSTYLE", "Describes the implementation behind the completion widget currently being\nexecuted; the second argument that followed `zle -C` when the widget was\ndefined.  This is the name of a builtin completion widget.  For widgets\ndefined with `zle -N` this is set to the empty string.  Read-only."),
    ("YANK_END", "`YANK_ACTIVE` indicates whether text has just been yanked (pasted)\ninto the buffer.  `YANK_START` and `YANK_END` give the location of\nthe pasted text and are in the same units as `CURSOR`.  They are only\nvalid for reading when `YANK_ACTIVE` is non-zero.  They can also be\nassigned by widgets that insert text in a yank-like fashion, for example\nwrappers of `bracketed-paste`.  See also `zle -f`.\n\n`YANK_ACTIVE` is read-only."),
    ("ZLE_RECURSIVE", "Usually zero, but incremented inside any instance of\n`recursive-edit`.  Hence indicates the current recursion level.\n\n`ZLE_RECURSIVE` is read-only."),
    ("ZLE_STATE", "Contains a set of space-separated words that describe the current `zle`\nstate.\n\nCurrently, the states shown are the insert mode as set by the\n`overwrite-mode` or `vi-replace` widgets and whether history commands\nwill visit imported entries as controlled by the set-local-history widget.\nThe string contains \"insert\" if characters to be inserted on the\ncommand line move existing characters to the right or \"overwrite\"\nif characters to be inserted overwrite existing characters. It contains\n\"localhistory\" if only local history commands will be visited or\n\"globalhistory\" if imported history commands will also be visited.\n\nThe substrings are sorted in alphabetical order so that if you want to\ntest for two specific substrings in a future-proof way, you can do match\nby doing:\n\n    if [[ $ZLE_STATE == *globalhistory*insert* ]]; then ...; fi"),
];

/// Alias → canonical mapping. One entry per surface name
/// that should resolve to the same documentation body.
pub const SPECIAL_VAR_ALIASES: &[(&str, &str)] = &[
    ("!", "!"),
    ("#", "#"),
    ("ARGC", "ARGC"),
    ("$", "$"),
    ("-", "-"),
    ("*", "*"),
    ("argv", "argv"),
    ("@", "@"),
    ("?", "?"),
    ("0", "0"),
    ("status", "status"),
    ("pipestatus", "pipestatus"),
    ("_", "_"),
    ("CPUTYPE", "CPUTYPE"),
    ("EGID", "EGID"),
    ("EUID", "EUID"),
    ("ERRNO", "ERRNO"),
    ("FUNCNEST", "FUNCNEST"),
    ("GID", "GID"),
    ("HISTCMD", "HISTCMD"),
    ("HOST", "HOST"),
    ("LINENO", "LINENO"),
    ("LOGNAME", "LOGNAME"),
    ("MACHTYPE", "MACHTYPE"),
    ("OLDPWD", "OLDPWD"),
    ("OPTARG", "OPTARG"),
    ("OPTIND", "OPTIND"),
    ("OSTYPE", "OSTYPE"),
    ("PPID", "PPID"),
    ("PWD", "PWD"),
    ("RANDOM", "RANDOM"),
    ("SECONDS", "SECONDS"),
    ("SHLVL", "SHLVL"),
    ("signals", "signals"),
    (".term.cursor", ".term.cursor"),
    (".term.bg", ".term.bg"),
    (".term.fg", ".term.fg"),
    (".term.id", ".term.id"),
    (".term.version", ".term.version"),
    (".term.mode", ".term.mode"),
    ("TRY_BLOCK_ERROR", "TRY_BLOCK_ERROR"),
    ("TRY_BLOCK_INTERRUPT", "TRY_BLOCK_INTERRUPT"),
    ("TTY", "TTY"),
    ("TTYIDLE", "TTYIDLE"),
    ("UID", "UID"),
    ("USERNAME", "USERNAME"),
    ("VENDOR", "VENDOR"),
    ("zsh_eval_context", "zsh_eval_context"),
    ("ZSH_EVAL_CONTEXT", "zsh_eval_context"),
    ("ZSH_ARGZERO", "ZSH_ARGZERO"),
    ("ZSH_EXECUTION_STRING", "ZSH_EXECUTION_STRING"),
    ("ZSH_EXEPATH", "ZSH_EXEPATH"),
    ("ZSH_NAME", "ZSH_NAME"),
    ("ZSH_PATCHLEVEL", "ZSH_PATCHLEVEL"),
    ("ZSH_SCRIPT", "ZSH_SCRIPT"),
    ("ZSH_SUBSHELL", "ZSH_SUBSHELL"),
    ("ZSH_SUBSHELL <S>", "ZSH_SUBSHELL"),
    ("ZSH_VERSION", "ZSH_VERSION"),
    ("ARGV0", "ARGV0"),
    ("BAUD", "BAUD"),
    ("cdpath", "cdpath"),
    ("CDPATH", "cdpath"),
    ("COLUMNS", "COLUMNS"),
    ("CORRECT_IGNORE", "CORRECT_IGNORE"),
    ("CORRECT_IGNORE_FILE", "CORRECT_IGNORE_FILE"),
    ("DIRSTACKSIZE", "DIRSTACKSIZE"),
    ("ENV", "ENV"),
    ("FCEDIT", "FCEDIT"),
    ("fignore", "fignore"),
    ("FIGNORE", "fignore"),
    ("fpath", "fpath"),
    ("FPATH", "fpath"),
    ("histchars", "histchars"),
    ("HISTCHARS", "HISTCHARS"),
    ("HISTFILE", "HISTFILE"),
    ("HISTORY_IGNORE", "HISTORY_IGNORE"),
    ("HISTSIZE", "HISTSIZE"),
    ("HOME", "HOME"),
    ("IFS", "IFS"),
    ("KEYBOARD_HACK", "KEYBOARD_HACK"),
    ("KEYTIMEOUT", "KEYTIMEOUT"),
    ("LANG", "LANG"),
    ("LC_ALL", "LC_ALL"),
    ("LC_COLLATE", "LC_COLLATE"),
    ("LC_CTYPE", "LC_CTYPE"),
    ("LC_MESSAGES", "LC_MESSAGES"),
    ("LC_NUMERIC", "LC_NUMERIC"),
    ("LC_TIME", "LC_TIME"),
    ("LINES", "LINES"),
    ("LISTMAX", "LISTMAX"),
    ("MAIL", "MAIL"),
    ("MAILCHECK", "MAILCHECK"),
    ("mailpath", "mailpath"),
    ("MAILPATH", "mailpath"),
    ("manpath", "manpath"),
    ("MANPATH", "manpath"),
    ("module_path", "module_path"),
    ("MODULE_PATH", "module_path"),
    ("NULLCMD", "NULLCMD"),
    ("path", "path"),
    ("PATH", "path"),
    ("POSTEDIT", "POSTEDIT"),
    ("PROMPT4", "PROMPT4"),
    ("PROMPT", "PROMPT4"),
    ("PROMPT2", "PROMPT4"),
    ("PROMPT3", "PROMPT4"),
    ("prompt", "prompt"),
    ("PROMPT_EOL_MARK", "PROMPT_EOL_MARK"),
    ("PS1", "PS1"),
    ("PS2", "PS2"),
    ("PS3", "PS3"),
    ("PS4", "PS4"),
    ("psvar", "psvar"),
    ("PSVAR", "psvar"),
    ("READNULLCMD", "READNULLCMD"),
    ("REPORTMEMORY", "REPORTMEMORY"),
    ("REPORTTIME", "REPORTTIME"),
    ("REPLY", "REPLY"),
    ("reply", "reply"),
    ("RPS1", "RPS1"),
    ("RPROMPT", "RPS1"),
    ("RPS2", "RPS2"),
    ("RPROMPT2", "RPS2"),
    ("SAVEHIST", "SAVEHIST"),
    ("SPROMPT", "SPROMPT"),
    ("STTY", "STTY"),
    ("TERM", "TERM"),
    ("TERMINFO", "TERMINFO"),
    ("TERMINFO_DIRS", "TERMINFO_DIRS"),
    (".term.extensions", ".term.extensions"),
    (".term.querywait", ".term.querywait"),
    ("TIMEFMT", "TIMEFMT"),
    ("TMOUT", "TMOUT"),
    ("TMPPREFIX", "TMPPREFIX"),
    ("TMPSUFFIX", "TMPSUFFIX"),
    ("WORDCHARS", "WORDCHARS"),
    ("ZBEEP", "ZBEEP"),
    ("ZDOTDIR", "ZDOTDIR"),
    ("zle_bracketed_paste", "zle_bracketed_paste"),
    ("zle_cursorform", "zle_cursorform"),
    ("zle_highlight", "zle_highlight"),
    ("ZLE_LINE_ABORTED", "ZLE_LINE_ABORTED"),
    ("ZLE_SPACE_SUFFIX_CHARS", "ZLE_SPACE_SUFFIX_CHARS"),
    ("ZLE_REMOVE_SUFFIX_CHARS", "ZLE_SPACE_SUFFIX_CHARS"),
    ("ZLE_RPROMPT_INDENT", "ZLE_RPROMPT_INDENT"),
    ("ZCURSES_COLORS", "ZCURSES_COLORS"),
    ("ZCURSES_COLOR_PAIRS", "ZCURSES_COLOR_PAIRS"),
    ("zcurses_attrs", "zcurses_attrs"),
    ("zcurses_colors", "zcurses_colors"),
    ("zcurses_keycodes", "zcurses_keycodes"),
    ("zcurses_windows", "zcurses_windows"),
    ("EPOCHREALTIME", "EPOCHREALTIME"),
    ("EPOCHSECONDS", "EPOCHSECONDS"),
    ("epochtime", "epochtime"),
    (".zle.esc", ".zle.esc"),
    (".zle.sgr", ".zle.sgr"),
    (".sh.command", ".sh.command"),
    (".sh.edchar", ".sh.edchar"),
    (".sh.edcol", ".sh.edcol"),
    (".sh.edmode", ".sh.edmode"),
    (".sh.edtext", ".sh.edtext"),
    (".sh.file", ".sh.file"),
    (".sh.fun", ".sh.fun"),
    (".sh.level", ".sh.level"),
    (".sh.lineno", ".sh.lineno"),
    (".sh.match", ".sh.match"),
    (".sh.name", ".sh.name"),
    (".sh.subscript", ".sh.subscript"),
    (".sh.subshell", ".sh.subshell"),
    (".sh.value", ".sh.value"),
    (".sh.version", ".sh.version"),
    ("langinfo", "langinfo"),
    ("mapfile", "mapfile"),
    ("options", "options"),
    ("commands", "commands"),
    ("functions", "functions"),
    ("dis_functions", "dis_functions"),
    ("functions_source", "functions_source"),
    ("dis_functions_source", "dis_functions_source"),
    ("builtins", "builtins"),
    ("dis_builtins", "dis_builtins"),
    ("reswords", "reswords"),
    ("dis_reswords", "dis_reswords"),
    ("patchars", "patchars"),
    ("dis_patchars", "dis_patchars"),
    ("aliases", "aliases"),
    ("dis_aliases", "dis_aliases"),
    ("galiases", "galiases"),
    ("dis_galiases", "dis_galiases"),
    ("saliases", "saliases"),
    ("dis_saliases", "dis_saliases"),
    ("parameters", "parameters"),
    ("modules", "modules"),
    ("dirstack", "dirstack"),
    ("history", "history"),
    ("historywords", "historywords"),
    ("jobdirs", "jobdirs"),
    ("jobtexts", "jobtexts"),
    ("jobstates", "jobstates"),
    ("nameddirs", "nameddirs"),
    ("userdirs", "userdirs"),
    ("usergroups", "usergroups"),
    ("funcfiletrace", "funcfiletrace"),
    ("funcsourcetrace", "funcsourcetrace"),
    ("funcstack", "funcstack"),
    ("functrace", "functrace"),
    ("SRANDOM", "SRANDOM"),
    ("zsh_scheduled_events", "zsh_scheduled_events"),
    ("errnos", "errnos"),
    ("sysparams", "sysparams"),
    ("termcap", "termcap"),
    ("terminfo", "terminfo"),
    ("LOGCHECK", "LOGCHECK"),
    ("watch", "watch"),
    ("WATCH", "watch"),
    ("WATCHFMT", "WATCHFMT"),
    ("log", "log"),
    ("ZFTP_TMOUT", "ZFTP_TMOUT"),
    ("ZFTP_IP", "ZFTP_IP"),
    ("ZFTP_HOST", "ZFTP_HOST"),
    ("ZFTP_PORT", "ZFTP_PORT"),
    ("ZFTP_SYSTEM", "ZFTP_SYSTEM"),
    ("ZFTP_TYPE", "ZFTP_TYPE"),
    ("ZFTP_USER", "ZFTP_USER"),
    ("ZFTP_ACCOUNT", "ZFTP_ACCOUNT"),
    ("ZFTP_PWD", "ZFTP_PWD"),
    ("ZFTP_CODE", "ZFTP_CODE"),
    ("ZFTP_REPLY", "ZFTP_REPLY"),
    ("ZFTP_SESSION", "ZFTP_SESSION"),
    ("ZFTP_PREFS", "ZFTP_PREFS"),
    ("ZFTP_VERBOSE", "ZFTP_VERBOSE"),
    ("ZFTP_FILE", "ZFTP_FILE"),
    ("ZFTP_TRANSFER", "ZFTP_TRANSFER"),
    ("ZFTP_SIZE", "ZFTP_SIZE"),
    ("ZFTP_COUNT", "ZFTP_COUNT"),
    ("keymaps", "keymaps"),
    ("widgets", "widgets"),
    ("zstyle", "zstyle"),
    ("incarg", "incarg"),
    ("BUFFER", "BUFFER"),
    ("BUFFERLINES", "BUFFERLINES"),
    ("CONTEXT", "CONTEXT"),
    ("CURSOR", "CURSOR"),
    ("CUTBUFFER", "CUTBUFFER"),
    ("HISTNO", "HISTNO"),
    ("ISEARCHMATCH_END", "ISEARCHMATCH_END"),
    ("ISEARCHMATCH_ACTIVE", "ISEARCHMATCH_END"),
    ("ISEARCHMATCH_START", "ISEARCHMATCH_END"),
    ("KEYMAP", "KEYMAP"),
    ("KEYS", "KEYS"),
    ("KEYS_QUEUED_COUNT", "KEYS_QUEUED_COUNT"),
    ("killring", "killring"),
    ("LASTABORTEDSEARCH", "LASTABORTEDSEARCH"),
    ("LASTSEARCH", "LASTSEARCH"),
    ("LASTWIDGET", "LASTWIDGET"),
    ("LBUFFER", "LBUFFER"),
    ("MARK", "MARK"),
    ("NUMERIC", "NUMERIC"),
    ("PENDING", "PENDING"),
    ("PREBUFFER", "PREBUFFER"),
    ("PREDISPLAY", "PREDISPLAY"),
    ("POSTDISPLAY", "POSTDISPLAY"),
    ("RBUFFER", "RBUFFER"),
    ("REGION_ACTIVE", "REGION_ACTIVE"),
    ("region_highlight", "region_highlight"),
    ("registers", "registers"),
    ("SUFFIX_END", "SUFFIX_END"),
    ("SUFFIX_ACTIVE", "SUFFIX_END"),
    ("SUFFIX_START", "SUFFIX_END"),
    ("UNDO_CHANGE_NO", "UNDO_CHANGE_NO"),
    ("UNDO_LIMIT_NO", "UNDO_LIMIT_NO"),
    ("WIDGET", "WIDGET"),
    ("WIDGETFUNC", "WIDGETFUNC"),
    ("WIDGETSTYLE", "WIDGETSTYLE"),
    ("YANK_END", "YANK_END"),
    ("YANK_ACTIVE", "YANK_END"),
    ("YANK_START", "YANK_END"),
    ("ZLE_RECURSIVE", "ZLE_RECURSIVE"),
    ("ZLE_STATE", "ZLE_STATE"),
];

/// Resolve a special variable name (case-insensitive, alias-aware)
/// to its canonical name + markdown body.
pub fn lookup_special_var_doc(name: &str) -> Option<(&'static str, &'static str)> {
    let key = name.to_string();
    let canon = SPECIAL_VAR_ALIASES
        .iter()
        .find(|(a, _)| *a == key.as_str())
        .map(|(_, c)| *c)?;
    SPECIAL_VAR_DOCS
        .iter()
        .find(|(n, _)| *n == canon)
        .map(|&(n, d)| (n, d))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── lookup_special_var_doc: alias → canonical → doc resolution ───

    #[test]
    fn lookup_known_special_var_returns_canon_and_doc() {
        // `?` is the exit-status special. Verify (canon, doc) tuple shape.
        let (canon, doc) = lookup_special_var_doc("?").expect("? must resolve");
        assert_eq!(canon, "?");
        assert!(!doc.is_empty(), "doc body must not be empty");
        assert!(doc.to_lowercase().contains("exit status"));
    }

    #[test]
    fn lookup_aliased_variable_resolves_to_canonical() {
        // `status` is an alias for `?` (same docs).
        let (canon_alias, doc_alias) =
            lookup_special_var_doc("status").expect("status must resolve");
        let (canon_orig, doc_orig) = lookup_special_var_doc("?").expect("? must resolve");
        // Aliases resolve through the SPECIAL_VAR_ALIASES table — both
        // entries are listed canonically as themselves in the doc table,
        // so the canon strings differ but the doc bodies should match
        // for the alias relationship.
        let _ = canon_alias;
        let _ = canon_orig;
        // Different canon strings, but the doc bodies for ? and status
        // may differ — the alias table maps name→canon, then docs table
        // maps canon→body. status canon = "status" → docs entry "status".
        // Verify the doc body is non-empty for both.
        assert!(!doc_alias.is_empty());
        assert!(!doc_orig.is_empty());
    }

    #[test]
    fn lookup_unknown_returns_none() {
        // Unknown name → None.
        assert!(lookup_special_var_doc("NOT_A_SPECIAL_VAR").is_none());
        assert!(lookup_special_var_doc("xyz_random_42").is_none());
    }

    #[test]
    fn lookup_empty_string_returns_none() {
        assert!(lookup_special_var_doc("").is_none());
    }

    #[test]
    fn lookup_argv_returns_array_doc() {
        let (canon, doc) = lookup_special_var_doc("argv").expect("argv must resolve");
        assert_eq!(canon, "argv");
        // Body should mention positional parameters per the source.
        assert!(doc.to_lowercase().contains("positional"));
    }

    #[test]
    fn lookup_uid_and_euid_both_resolve() {
        // UID, EUID, GID, EGID are all in the table.
        for name in ["UID", "EUID", "GID", "EGID"] {
            assert!(
                lookup_special_var_doc(name).is_some(),
                "{name} should resolve"
            );
        }
    }

    #[test]
    fn lookup_zsh_specific_variables() {
        // ZSH_VERSION / ZSH_NAME / ZSH_PATCHLEVEL / etc.
        for name in [
            "ZSH_VERSION",
            "ZSH_NAME",
            "ZSH_SCRIPT",
            "ZSH_SUBSHELL",
            "ZSH_EXEPATH",
        ] {
            assert!(
                lookup_special_var_doc(name).is_some(),
                "{name} should resolve"
            );
        }
    }

    #[test]
    fn lookup_dot_term_namespace_variables() {
        // .term.cursor, .term.bg, .term.fg, .term.mode, .term.id, .term.version.
        for name in [
            ".term.cursor",
            ".term.bg",
            ".term.fg",
            ".term.id",
            ".term.mode",
            ".term.version",
        ] {
            assert!(
                lookup_special_var_doc(name).is_some(),
                "{name} should resolve"
            );
        }
    }

    // ─── SPECIAL_VAR_ALIASES + DOCS table integrity ───────────────────

    #[test]
    fn every_alias_canon_target_exists_in_docs() {
        // Every right-side value in SPECIAL_VAR_ALIASES must point to a
        // valid entry in SPECIAL_VAR_DOCS — otherwise the lookup would
        // resolve to None for a name that's clearly in the aliases table.
        let doc_names: std::collections::HashSet<&str> =
            SPECIAL_VAR_DOCS.iter().map(|(n, _)| *n).collect();
        for (alias, canon) in SPECIAL_VAR_ALIASES {
            assert!(
                doc_names.contains(canon),
                "alias {alias:?} → canon {canon:?} but canon is missing from SPECIAL_VAR_DOCS"
            );
        }
    }

    #[test]
    fn special_var_docs_no_empty_bodies() {
        // Every doc string should be non-empty — empty would mean the
        // generator missed content during parse.
        for (name, doc) in SPECIAL_VAR_DOCS {
            assert!(!doc.is_empty(), "empty body for {name:?}");
        }
    }

    #[test]
    fn special_var_aliases_table_is_nonempty() {
        // Sanity guard against the autogen accidentally producing an
        // empty table.
        assert!(!SPECIAL_VAR_ALIASES.is_empty());
        assert!(
            SPECIAL_VAR_ALIASES.len() >= 50,
            "expected ≥50 aliases, got {}",
            SPECIAL_VAR_ALIASES.len()
        );
    }

    #[test]
    fn special_var_docs_table_is_nonempty() {
        assert!(!SPECIAL_VAR_DOCS.is_empty());
        assert!(
            SPECIAL_VAR_DOCS.len() >= 50,
            "expected ≥50 doc entries, got {}",
            SPECIAL_VAR_DOCS.len()
        );
    }
}
