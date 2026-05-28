package com.menketechnologies.zshrs

import com.menketechnologies.zshrs.ZshrsSmartEnterProcessor.Companion.computePlan
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * Pure JUnit 4 tests for [ZshrsSmartEnterProcessor.computePlan].
 *
 * In every test the `src` parameter uses `|` to mark the user's
 * caret position (stripped before the planner is called); `expected`
 * inserts a `|` where the caret should land after the plan applies.
 */
class ZshrsSmartEnterProcessorTest {
    private fun caretOf(src: String): Pair<String, Int> {
        val i = src.indexOf('|')
        require(i >= 0) { "test fixture must contain '|' for caret: $src" }
        return src.removeRange(i, i + 1) to i
    }

    private fun applyPlan(src: String): String {
        val (text, caret) = caretOf(src)
        val lineStart = text.lastIndexOf('\n', caret - 1).let { if (it < 0) 0 else it + 1 }
        val lineEnd = text.indexOf('\n', caret).let { if (it < 0) text.length else it }
        val line = text.substring(lineStart, lineEnd)
        val plan = computePlan(line, lineStart, caret, text)
            ?: error("expected a plan for: $src")
        val sb = StringBuilder(text)
        sb.insert(plan.offset, plan.insert)
        sb.insert(plan.offset + plan.caretRel, "|")
        return sb.toString()
    }

    private fun assertPlan(src: String, expected: String) {
        assertEquals(expected, applyPlan(src))
    }

    private fun assertNoPlan(src: String) {
        val (text, caret) = caretOf(src)
        val lineStart = text.lastIndexOf('\n', caret - 1).let { if (it < 0) 0 else it + 1 }
        val lineEnd = text.indexOf('\n', caret).let { if (it < 0) text.length else it }
        val line = text.substring(lineStart, lineEnd)
        assertNull(
            "expected no plan for: $src",
            computePlan(line, lineStart, caret, text),
        )
    }

    // ── Strategy 1: control-flow blocks ────────────────────────────

    @Test fun if_with_test_expression_completes_then_fi() {
        assertPlan(
            "if [[ \$x = y ]]|",
            "if [[ \$x = y ]]; then\n    |\nfi",
        )
    }

    @Test fun if_with_already_typed_then_just_adds_body_and_fi() {
        assertPlan(
            "if [[ \$x ]]; then|",
            "if [[ \$x ]]; then\n    |\nfi",
        )
    }

    @Test fun if_with_unclosed_test_bracket_is_noop() {
        // `[[` not yet closed — the bracket-balance strategy fires
        // instead. Control-flow header isn't ready yet.
        assertEquals(
            "if [[ \$x = y]]|",
            applyPlan("if [[ \$x = y|"),
        )
    }

    @Test fun while_loop_completes_do_done() {
        assertPlan(
            "while [[ -n \$line ]]|",
            "while [[ -n \$line ]]; do\n    |\ndone",
        )
    }

    @Test fun until_loop_completes_do_done() {
        assertPlan(
            "until \$ready|",
            "until \$ready; do\n    |\ndone",
        )
    }

    @Test fun for_loop_completes_do_done() {
        assertPlan(
            "for x in 1 2 3|",
            "for x in 1 2 3; do\n    |\ndone",
        )
    }

    @Test fun select_completes_do_done() {
        assertPlan(
            "select item in \"\${entries[@]}\"|",
            "select item in \"\${entries[@]}\"; do\n    |\ndone",
        )
    }

    @Test fun repeat_completes_do_done() {
        assertPlan(
            "repeat 10|",
            "repeat 10; do\n    |\ndone",
        )
    }

    @Test fun elif_completes_then_no_closer() {
        // `elif` is part of an enclosing `if`, so no `fi` is added.
        assertPlan(
            "elif [[ \$y ]]|",
            "elif [[ \$y ]]; then\n    |",
        )
    }

    @Test fun else_just_adds_indent_line() {
        assertPlan(
            "else|",
            "else\n    |",
        )
    }

    @Test fun case_with_bare_subject_adds_in_and_esac() {
        assertPlan(
            "case \$x|",
            "case \$x in\n    |\nesac",
        )
    }

    @Test fun case_with_in_already_typed_adds_body_and_esac() {
        assertPlan(
            "case \$x in|",
            "case \$x in\n    |\nesac",
        )
    }

    @Test fun control_block_preserves_indent() {
        assertPlan(
            "    if [[ \$x ]]|",
            "    if [[ \$x ]]; then\n        |\n    fi",
        )
    }

    @Test fun control_block_skips_when_closer_already_present() {
        // User already has `fi` further down — don't add a second one.
        assertNoPlan(
            "if [[ \$x ]]|\nfi",
        )
    }

    // ── Strategy 2: function declarations ──────────────────────────

    @Test fun function_keyword_with_name_adds_parens_and_body() {
        assertPlan(
            "function greet|",
            "function greet() {\n    |\n}",
        )
    }

    @Test fun function_keyword_with_name_and_parens_adds_body() {
        assertPlan(
            "function greet()|",
            "function greet() {\n    |\n}",
        )
    }

    @Test fun posix_function_decl_adds_body() {
        assertPlan(
            "greet()|",
            "greet() {\n    |\n}",
        )
    }

    @Test fun function_with_body_already_started_is_noop() {
        assertNoPlan("function greet() {|\n}")
    }

    @Test fun function_preserves_indent() {
        assertPlan(
            "  function inner|",
            "  function inner() {\n      |\n  }",
        )
    }

    // ── Strategy 3: bracket / quote balance ────────────────────────

    @Test fun unclosed_double_bracket_closes_with_double_close() {
        assertPlan(
            "[[ -f /tmp/foo|",
            "[[ -f /tmp/foo]]|",
        )
    }

    @Test fun unclosed_double_paren_arith_closes_with_double_close() {
        assertPlan(
            "(( i + 1|",
            "(( i + 1))|",
        )
    }

    @Test fun unclosed_single_paren_subshell_closes_with_single() {
        assertPlan(
            "echo \$(date|",
            "echo \$(date)|",
        )
    }

    @Test fun unclosed_single_bracket_closes_with_single() {
        assertPlan(
            "[ -f /tmp/foo|",
            "[ -f /tmp/foo]|",
        )
    }

    @Test fun nested_brackets_close_in_correct_order() {
        // Outer `((` (needs `))`) wraps inner `(` (needs `)`). Inner
        // closes first, then outer — `)` + `))` = three close parens.
        assertPlan(
            "(( a + (b * 2|",
            "(( a + (b * 2)))|",
        )
    }

    @Test fun balanced_line_is_noop() {
        assertNoPlan("[[ \$x ]]|")
    }

    @Test fun unclosed_paren_inside_string_is_ignored() {
        assertNoPlan("echo \"open (paren|\"")
    }

    @Test fun balance_inserts_before_trailing_comment() {
        // `]]` must land before the `# note`, not after.
        assertPlan(
            "[[ -f \$f|  # check file",
            "[[ -f \$f]]|  # check file",
        )
    }

    // ── Strategy 4: bare body openers ──────────────────────────────

    @Test fun then_at_end_of_line_adds_body_and_fi() {
        // User typed `if X; then` on its own line. Smart-enter drops
        // the body indent + matching `fi`.
        assertPlan(
            "if grep -q foo /tmp/x; then|",
            "if grep -q foo /tmp/x; then\n    |\nfi",
        )
    }

    @Test fun do_at_end_of_line_adds_body_and_done() {
        assertPlan(
            "while read line; do|",
            "while read line; do\n    |\ndone",
        )
    }

    @Test fun bare_in_after_case_adds_body_and_esac() {
        // Same as case_with_in_already_typed but exercises the
        // bare-opener path (no leading `case` on the SAME line of
        // the caret — the `case` ends on a previous line).
        assertPlan(
            "case \$x in|",
            "case \$x in\n    |\nesac",
        )
    }

    // ── Misc edge cases ────────────────────────────────────────────

    @Test fun comment_line_is_noop() {
        assertNoPlan("# while loop coming|")
    }

    @Test fun blank_line_is_noop() {
        assertNoPlan("|")
    }
}
