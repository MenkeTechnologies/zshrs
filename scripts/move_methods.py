#!/usr/bin/env python3
"""Generic bulk-mover for ShellExecutor methods.

Reads a TARGETS dict ({method_name: dest_rel_path}) from a JSON sidecar
file, then walks src/ported/exec.rs and:

* Captures each named method block (with preceding doc/attr lines).
* Skips occurrences inside `impl Trait for X` blocks (visibility
  qualifiers are illegal there).
* Bumps each captured method to `pub(crate)` if it had no qualifier.
* Removes the matched blocks from exec.rs and appends them to each
  destination file inside a fresh `impl crate::ported::exec::ShellExecutor`
  block guarded by a `// BEGIN moved-from-exec-rs` marker.

Usage:
    python3 scripts/move_methods.py PHASE_NAME

where PHASE_NAME is a key in PHASES below.
"""
from __future__ import annotations
import json
import re
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
EXEC = ROOT / "src/ported/exec.rs"

PHASES: dict[str, dict[str, str]] = {
    # =================================================================
    # subst.c (4922 LOC) -> src/ported/subst.rs
    # =================================================================
    "subst": {
        "expand_string":                  "src/ported/subst.rs",
        "expand_string_split":            "src/ported/subst.rs",
        "expand_word":                    "src/ported/subst.rs",
        "expand_concat_parallel":         "src/ported/subst.rs",
        "expand_braces":                  "src/ported/subst.rs",
        "expand_brace_sequence":          "src/ported/subst.rs",
        "expand_brace_ccl":               "src/ported/subst.rs",
        "expand_brace_list":              "src/ported/subst.rs",
        "expand_tilde_named":             "src/ported/subst.rs",
        "expand_extglob":                 "src/ported/subst.rs",
        "expand_neg_extglob":             "src/ported/subst.rs",
        "apply_zsh_param_flag":           "src/ported/subst.rs",
        "parse_zsh_flags":                "src/ported/subst.rs",
        "apply_var_modifier":             "src/ported/subst.rs",
    },
    # =================================================================
    # Module shim phase: each builtin_* shim moves to its module file.
    # =================================================================
    "module-shims": {
        "builtin_ulimit":                 "src/ported/builtins/rlimits.rs",
        "builtin_limit":                  "src/ported/builtins/rlimits.rs",
        "builtin_unlimit":                "src/ported/builtins/rlimits.rs",
        "builtin_sched":                  "src/ported/builtins/sched.rs",
        "builtin_strftime":               "src/ported/modules/datetime.rs",
        "builtin_zselect":                "src/ported/modules/zselect.rs",
        "builtin_echotc":                 "src/ported/modules/termcap.rs",
        "builtin_zpty":                   "src/ported/modules/zpty.rs",
        "builtin_zprof":                  "src/ported/modules/zprof.rs",
        "builtin_zsocket":                "src/ported/modules/socket.rs",
        "builtin_ztcp":                   "src/ported/modules/tcp.rs",
        "builtin_clone":                  "src/ported/modules/clone.rs",
        "builtin_log":                    "src/ported/modules/watch.rs",
        "builtin_cap":                    "src/ported/modules/cap.rs",
        "builtin_getcap":                 "src/ported/modules/cap.rs",
        "builtin_setcap":                 "src/ported/modules/cap.rs",
        "builtin_zcurses":                "src/ported/modules/curses.rs",
        "builtin_private":                "src/ported/modules/param_private.rs",
        "builtin_zftp":                   "src/ported/modules/zftp.rs",
        "builtin_pcre_compile":           "src/ported/modules/pcre.rs",
        "builtin_pcre_match":             "src/ported/modules/pcre.rs",
        "builtin_pcre_study":             "src/ported/modules/pcre.rs",
    },
    # =================================================================
    # pattern.c -> src/ported/pattern.rs
    # =================================================================
    "pattern": {
        "extglob_to_regex":               "src/ported/pattern.rs",
        "extglob_inner_to_regex":         "src/ported/pattern.rs",
        "extract_extglob_inner":          "src/ported/pattern.rs",
        "extract_neg_extglob":            "src/ported/pattern.rs",
        "has_extglob_pattern":            "src/ported/pattern.rs",
    },
    # =================================================================
    # glob.c -> src/ported/glob.rs
    # =================================================================
    "glob": {
        "expand_glob":                    "src/ported/glob.rs",
        "expand_glob_parallel":           "src/ported/glob.rs",
        "expand_glob_with_numeric_range": "src/ported/glob.rs",
        "filter_by_qualifiers":           "src/ported/glob.rs",
        "glob_match_static":              "src/ported/glob.rs",
        "glob_match":                     "src/ported/glob.rs",
        "parse_glob_qualifiers":          "src/ported/glob.rs",
        "looks_like_glob":                "src/ported/glob.rs",
        "looks_like_glob_qualifiers":     "src/ported/glob.rs",
        "has_balanced_escaped_braces":    "src/ported/glob.rs",
        "matches_pattern":                "src/ported/glob.rs",
    },
    # =================================================================
    # math.c -> src/ported/math.rs
    # =================================================================
    "math": {
        "evaluate_arithmetic":            "src/ported/math.rs",
        "evaluate_arithmetic_expr":       "src/ported/math.rs",
        "execarith":                      "src/ported/math.rs",
        "eval_arith_expr":                "src/ported/math.rs",
        "eval_arith_expr_float":          "src/ported/math.rs",
    },
    # =================================================================
    # params.c -> src/ported/params.rs
    # =================================================================
    "params": {
        "get_special_array_value":        "src/ported/params.rs",
        "get_variable":                   "src/ported/params.rs",
        "lookup_array_element":           "src/ported/params.rs",
        "array_element_is_set":           "src/ported/params.rs",
        "pre_resolve_array_subscripts":   "src/ported/params.rs",
        "pre_resolve_dollar_subscripts":  "src/ported/params.rs",
        "parse_subscript_range":          "src/ported/params.rs",
        "format_for_var_attr":            "src/ported/params.rs",
        "split_words":                    "src/ported/params.rs",
    },
    # =================================================================
    # hist.c -> src/ported/hist.rs
    # =================================================================
    "hist": {
        "expand_history":                 "src/ported/hist.rs",
        "apply_history_modifiers":        "src/ported/hist.rs",
        "is_history_modifier":            "src/ported/hist.rs",
        "history_split_words":            "src/ported/hist.rs",
        "history_quick_subst":            "src/ported/hist.rs",
        "history_resolve_event":          "src/ported/hist.rs",
        "history_apply_designators_and_modifiers": "src/ported/hist.rs",
        "history_parse_word_range":       "src/ported/hist.rs",
    },
    # =================================================================
    # builtin.c -> src/ported/builtin.rs (printf, cd, dirs, reserved kw)
    # =================================================================
    "builtin-print": {
        "printf_format_count":            "src/ported/builtin.rs",
        "printf_format":                  "src/ported/builtin.rs",
        "expand_printf_escapes":          "src/ported/builtin.rs",
        "expand_printf_escapes_internal_marker": "src/ported/builtin.rs",
        "expand_print_escapes":           "src/ported/builtin.rs",
        "do_cd":                          "src/ported/builtin.rs",
        "print_dir_stack":                "src/ported/builtin.rs",
        "sync_dirstack_array":            "src/ported/builtin.rs",
        "is_reserved_word":               "src/ported/builtin.rs",
    },
    # =================================================================
    # prompt.c -> src/ported/prompt.rs
    # =================================================================
    "prompt": {
        "apply_prompt_theme":             "src/ported/prompt.rs",
        "expand_prompt_string":           "src/ported/prompt.rs",
        "expand_prompt_string_for_print": "src/ported/prompt.rs",
        "expand_bindkey_escapes":         "src/ported/prompt.rs",
    },
    # =================================================================
    # autoload (loadautofn lives in exec.c; bin_autoload in builtin.c;
    # autoloadscan in module.c). We send the loaders to builtin.rs
    # since that's where bin_autoload is implemented.
    # =================================================================
    "autoload": {
        "load_autoload_function":         "src/ported/builtin.rs",
        "load_function_from_zwc":         "src/ported/builtin.rs",
        "autoload_function":              "src/ported/builtin.rs",
        "maybe_autoload":                 "src/ported/builtin.rs",
    },
    "drift": {
        "magic_assoc_lookup":             "src/ported/params.rs",
        "prefetch_metadata":              "src/ported/glob.rs",
        "filter_by_permission":           "src/ported/glob.rs",
        "get_psvar":                      "src/ported/prompt.rs",
        "get_term_width":                 "src/ported/prompt.rs",
        "get_builtin_names":              "src/ported/builtin.rs",
        "find_function_file":             "src/ported/builtin.rs",
        "add_named_dir":                  "src/ported/hashnameddir.rs",
        "run_trap":                       "src/ported/signals.rs",
        "copy_dir_recursive":             "src/ported/utils.rs",
        "format_zsh":                     "src/ported/text.rs",
        "recorder_attrs_for":             "src/extensions/recorder.rs",
        "recorder_ctx":                   "src/extensions/recorder.rs",
        "execute_advice":                 "src/extensions/intercepts.rs",
        "run_original_command":           "src/extensions/intercepts.rs",
        "run_intercepts":                 "src/extensions/intercepts.rs",
        "snapshot_state":                 "src/extensions/plugin_cache.rs",
        "diff_state":                     "src/extensions/plugin_cache.rs",
        "replay_plugin_delta":            "src/extensions/plugin_cache.rs",
        "drain_compinit_bg":              "src/extensions/compinit_bg.rs",
        "compinit_compat":                "src/extensions/compinit_bg.rs",
        "run_hooks":                      "src/extensions/hooks.rs",
        "add_hook":                       "src/extensions/hooks.rs",
    },
    # =================================================================
    # Drift relocation by canonical C file
    # =================================================================
    "drift": {
        # params.c
        "magic_assoc_lookup":             "src/ported/params.rs",
        # glob.c
        "prefetch_metadata":              "src/ported/glob.rs",
        "filter_by_permission":           "src/ported/glob.rs",
        # prompt.c
        "get_psvar":                      "src/ported/prompt.rs",
        "get_term_width":                 "src/ported/prompt.rs",
        # builtin.c
        "get_builtin_names":              "src/ported/builtin.rs",
        "find_function_file":             "src/ported/builtin.rs",
        # hashnameddir.c
        "add_named_dir":                  "src/ported/hashnameddir.rs",
        # signals.c
        "run_trap":                       "src/ported/signals.rs",
        # utils.c
        "copy_dir_recursive":             "src/ported/utils.rs",
        # text.c
        "format_zsh":                     "src/ported/text.rs",
        # extensions (no C equivalent)
        "recorder_attrs_for":             "src/extensions/recorder.rs",
        "recorder_ctx":                   "src/extensions/recorder.rs",
        "execute_advice":                 "src/extensions/intercepts.rs",
        "run_original_command":           "src/extensions/intercepts.rs",
        "run_intercepts":                 "src/extensions/intercepts.rs",
        "snapshot_state":                 "src/extensions/plugin_cache.rs",
        "diff_state":                     "src/extensions/plugin_cache.rs",
        "replay_plugin_delta":            "src/extensions/plugin_cache.rs",
        "drain_compinit_bg":              "src/extensions/compinit_bg.rs",
        "compinit_compat":                "src/extensions/compinit_bg.rs",
        "run_hooks":                      "src/extensions/hooks.rs",
        "add_hook":                       "src/extensions/hooks.rs",
    },
}

# Generic signature: any fn name (4-letter identifier or longer is fine).
SIG_RE = re.compile(
    r"^(    )(pub(?:\(crate\))? )?(fn ([a-zA-Z_][a-zA-Z0-9_]*))\b"
)
ATTR_OR_DOC_RE = re.compile(r"^    (?://|///|#\[)")
END_RE = re.compile(r"^    \}\s*$")
# A one-liner body `{}` on the same line as the signature: `fn x() {}`.
ONE_LINER_RE = re.compile(r"\{\s*\}\s*$")
IMPL_OPEN_RE = re.compile(r"^impl(?:<[^>]*>)?\s+(.+?)\s*\{")
MARKER = "// BEGIN moved-from-exec-rs\n"


def find_blocks(lines, targets: dict[str, str]):
    blocks = []
    in_trait_impl = False
    in_shellexec_impl = False
    impl_depth = 0
    for i, line in enumerate(lines):
        if not line.startswith(" ") and not line.startswith("\t"):
            m = IMPL_OPEN_RE.match(line)
            if m:
                head = m.group(1)
                in_trait_impl = " for " in head
                # Only inherent impls on ShellExecutor are eligible.
                if in_trait_impl:
                    in_shellexec_impl = head.split(" for ", 1)[1].strip().startswith("ShellExecutor")
                else:
                    in_shellexec_impl = head.strip().startswith("ShellExecutor")
                impl_depth = 1
                continue
            if line.rstrip("\n") == "}" and impl_depth > 0:
                impl_depth = 0
                in_trait_impl = False
                in_shellexec_impl = False
                continue
        m = SIG_RE.match(line)
        if not m:
            continue
        name = m.group(4)
        if name not in targets:
            continue
        if not in_shellexec_impl:
            continue
        start = i
        j = i - 1
        while j >= 0 and ATTR_OR_DOC_RE.match(lines[j]):
            start = j
            j -= 1
        # Same-line one-liner body: `fn x() {}` ends on the signature line itself.
        if ONE_LINER_RE.search(line):
            blocks.append((start, i, name, in_trait_impl))
            continue
        end = None
        for k in range(i + 1, len(lines)):
            if END_RE.match(lines[k]):
                end = k
                break
        if end is None:
            raise RuntimeError(f"No closing brace for {name} at line {i+1}")
        blocks.append((start, end, name, in_trait_impl))
    return blocks


def run_phase(phase_name: str):
    targets = PHASES[phase_name]
    src = EXEC.read_text()
    lines = src.splitlines(keepends=True)
    blocks = find_blocks(lines, targets)
    found = {b[2] for b in blocks}
    missing = set(targets) - found
    if missing:
        print(f"WARNING: {len(missing)} target methods not found in exec.rs:")
        for n in sorted(missing):
            print(f"  {n}")
    print(f"phase={phase_name}: found {len(blocks)} method blocks (targets={len(targets)})")

    trait_blocks = [b for b in blocks if b[3]]
    if trait_blocks:
        print(f"WARNING: {len(trait_blocks)} matches inside trait impls -- skipping:")
        for b in trait_blocks:
            print(f"  {b[2]} at line {b[0]+1}")
    blocks = [b for b in blocks if not b[3]]

    extracted_per_dest: dict[str, list[str]] = defaultdict(list)
    for start, end, name, _ in sorted(blocks, key=lambda b: b[0]):
        chunk = "".join(lines[start:end + 1])
        new_lines = []
        for ln in chunk.splitlines(keepends=True):
            m = SIG_RE.match(ln)
            if m and (m.group(2) is None or m.group(2).strip() == ""):
                new_lines.append(f"{m.group(1)}pub(crate) {m.group(3)}{ln[m.end():]}")
            else:
                new_lines.append(ln)
        extracted_per_dest[targets[name]].append("".join(new_lines))

    for start, end, _, _ in sorted(blocks, key=lambda b: b[0], reverse=True):
        end_strip = end + 1
        if end_strip < len(lines) and lines[end_strip].strip() == "":
            end_strip += 1
        del lines[start:end_strip]

    EXEC.write_text("".join(lines))
    print(f"removed {len(blocks)} methods from exec.rs")

    for dest_rel, chunks in sorted(extracted_per_dest.items()):
        dest_path = ROOT / dest_rel
        dest_text = dest_path.read_text()
        additions = []
        additions.append("\n")
        additions.append("// ===========================================================\n")
        additions.append("// Methods moved verbatim from src/ported/exec.rs because their\n")
        additions.append(f"// C counterpart's source file maps 1:1 to this Rust module.\n")
        additions.append(f"// Phase: {phase_name}\n")
        additions.append("// ===========================================================\n")
        additions.append("\n")
        additions.append(MARKER)
        additions.append("impl crate::ported::exec::ShellExecutor {\n")
        for chunk in chunks:
            additions.append(chunk)
        additions.append("}\n")
        additions.append("// END moved-from-exec-rs\n")
        dest_path.write_text(dest_text + "".join(additions))
        print(f"appended {len(chunks):>2} methods -> {dest_rel}")


def main():
    if len(sys.argv) != 2 or sys.argv[1] not in PHASES:
        print(f"usage: {sys.argv[0]} <{'|'.join(PHASES)}>")
        sys.exit(2)
    run_phase(sys.argv[1])


if __name__ == "__main__":
    main()
