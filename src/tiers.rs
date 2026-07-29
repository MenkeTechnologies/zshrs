//! Which fusevm execution tier a program's bytecode actually reaches.
//!
//! Enabling the JIT is not the same as being compiled by it, and the only
//! honest way to tell the two apart is to ask the VM. This module runs a
//! program and then queries fusevm's own eligibility and cache predicates —
//! `is_block_eligible`, `block_jit_is_compiled`, `trace_is_compiled`,
//! `find_jit_region` — so the answer comes from the compiler that would have
//! done the work rather than from an assumption about it.
//!
//! `zshrs --tiers script.zsh` prints the report.

use std::collections::BTreeMap;

use fusevm::{Chunk, ChunkBuilder, JitCompiler, Op};

/// A loop header — the target of a backward branch — and what became of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loop {
    /// Op index of the loop header the backward branch jumps to.
    pub anchor: usize,
    /// Whether fusevm would accept this loop's body as a trace. Asked of
    /// `is_trace_eligible` with the body's ops — the same predicate the
    /// recorder applies to what it recorded, which for a loop whose body has
    /// no early exit is the same op sequence.
    pub trace_eligible: bool,
    /// Whether a compiled trace is installed for this header after the run.
    pub traced: bool,
    /// Whether the tracing JIT gave up on this header.
    pub blacklisted: bool,
}

/// What the tiers did with one chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkTiers {
    /// Which chunk this is — `main` for a whole zsh program.
    pub name: String,
    /// Ops in the compiled chunk.
    pub ops: usize,
    /// Whether every op in the chunk is block-JIT eligible, which is what the
    /// whole-chunk block tier requires.
    pub block_eligible: bool,
    /// Whether the block tier holds compiled native code for this chunk.
    pub block_compiled: bool,
    /// The largest contiguous block-eligible op range, if any is large enough
    /// for fusevm to consider it worth compiling.
    pub largest_eligible_region: Option<(usize, usize)>,
    /// Every loop header, and whether the tracing JIT compiled it.
    pub loops: Vec<Loop>,
    /// Op kinds the **block** tier refuses, by occurrence count — what keeps
    /// the whole chunk from being compiled in one piece.
    ///
    /// Not the same question as whether a loop is traced: the tracing tier
    /// takes `GetVar` / `SetVar` (fusevm promotes a referenced global to a
    /// register at trace entry and spills it at every exit), so a chunk can
    /// list those here and still reach native code through a trace.
    pub ineligible: BTreeMap<String, usize>,
}

impl ChunkTiers {
    /// Whether any tier holds compiled native code for this chunk.
    pub fn reaches_native(&self) -> bool {
        self.block_compiled || self.loops.iter().any(|l| l.traced)
    }
}

/// What the tiers did with one program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// Every chunk the program compiled to, in the order they were lowered.
    pub chunks: Vec<ChunkTiers>,
}

impl Report {
    /// Whether any tier holds compiled native code for any of the program's
    /// chunks.
    pub fn reaches_native(&self) -> bool {
        self.chunks.iter().any(|c| c.reaches_native())
    }
}

impl std::fmt::Display for ChunkTiers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "ops                     {}", self.ops)?;
        writeln!(f, "block-JIT eligible      {}", self.block_eligible)?;
        writeln!(f, "block-JIT compiled      {}", self.block_compiled)?;
        match self.largest_eligible_region {
            Some((s, e)) => writeln!(f, "largest eligible region {s}..{e} ({} ops)", e - s)?,
            None => writeln!(f, "largest eligible region none")?,
        }
        if self.loops.is_empty() {
            writeln!(f, "loops                   none")?;
        }
        for l in &self.loops {
            writeln!(
                f,
                "loop @{:<4}             trace-eligible={} traced={} blacklisted={}",
                l.anchor, l.trace_eligible, l.traced, l.blacklisted
            )?;
        }
        if self.ineligible.is_empty() {
            writeln!(f, "block-ineligible ops    none")?;
        } else {
            writeln!(f, "block-ineligible ops")?;
            for (name, count) in &self.ineligible {
                writeln!(f, "  {name:<22}{count}")?;
            }
        }
        Ok(())
    }
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // A single-chunk program needs no section headers; the fleet's
        // multi-chunk frontends label each one.
        let label = self.chunks.len() > 1;
        for c in &self.chunks {
            if label {
                writeln!(f, "== {} ==", c.name)?;
            }
            write!(f, "{c}")?;
        }
        write!(f, "reaches native code     {}", self.reaches_native())
    }
}

/// Compile and run `src`, then report which tiers took each of its chunks.
///
/// The script is run because tier membership is a runtime fact: the block tier
/// compiles after its warmup threshold and the tracing tier only after a loop
/// has gone round enough times to be recorded. zshrs writes script output
/// straight to the process stdout, so the script's own output precedes the
/// report — what is measured is what an ordinary run does.
///
/// The chunk lowered here is the one `execute_script_zsh_pipeline` lowers (same
/// parse, same `ZshCompiler`), and fusevm keys its compiled code by the chunk's
/// op hash, so asking this copy is asking about the run that just happened.
/// Function bodies are compiled on first dispatch and kept on the executor, so
/// they are collected from it afterwards — a function never called during the
/// run has no chunk yet and is correctly absent.
pub fn report(src: &str) -> Result<Report, String> {
    let program = crate::vm_helper::parse_isolated(src);
    let main = crate::compile_zsh::ZshCompiler::new().compile(&program);

    let mut exec = crate::vm_helper::ShellExecutor::new();
    exec.execute_script_zsh_pipeline(src)?;

    let mut named = vec![("main".to_string(), main)];
    let mut funcs: Vec<(String, Chunk)> = exec
        .functions_compiled
        .iter()
        .map(|(name, chunk)| (format!("function {name}"), chunk.clone()))
        .collect();
    // `functions_compiled` is a HashMap, so fix an order the report can be
    // diffed across runs.
    funcs.sort_by(|a, b| a.0.cmp(&b.0));
    named.extend(funcs);
    Ok(inspect_all(&named))
}


/// Report on one already-executed chunk, as a whole-program report. Used by
/// tests that build a chunk by hand.
pub fn inspect(chunk: &Chunk) -> Report {
    Report {
        chunks: vec![inspect_chunk("main", chunk)],
    }
}

/// Report on every chunk a compiled program holds, in lowering order.
pub fn inspect_all(named: &[(String, Chunk)]) -> Report {
    Report {
        chunks: named.iter().map(|(n, c)| inspect_chunk(n, c)).collect(),
    }
}

/// Report on one already-executed chunk.
pub fn inspect_chunk(name: &str, chunk: &Chunk) -> ChunkTiers {
    let jit = JitCompiler::new();
    let loops = loop_anchors(&chunk.ops)
        .into_iter()
        .map(|anchor| Loop {
            anchor,
            trace_eligible: body_of(&chunk.ops, anchor)
                .is_some_and(|body| jit.is_trace_eligible(body, anchor)),
            traced: jit.trace_is_compiled(chunk, anchor),
            blacklisted: jit.trace_is_blacklisted(chunk, anchor),
        })
        .collect();

    let mut ineligible: BTreeMap<String, usize> = BTreeMap::new();
    for op in &chunk.ops {
        if !op_is_eligible(&jit, op) {
            *ineligible.entry(op_name(op)).or_default() += 1;
        }
    }

    ChunkTiers {
        name: name.to_string(),
        ops: chunk.ops.len(),
        block_eligible: jit.is_block_eligible(chunk),
        block_compiled: jit.block_jit_is_compiled(chunk),
        largest_eligible_region: jit.find_jit_region(chunk),
        loops,
        ineligible,
    }
}

/// Every op index a backward branch jumps to — fusevm anchors a trace at each.
fn loop_anchors(ops: &[Op]) -> Vec<usize> {
    let mut anchors: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter_map(|(ip, op)| match op {
            Op::Jump(t)
            | Op::JumpIfTrue(t)
            | Op::JumpIfFalse(t)
            | Op::JumpIfTrueKeep(t)
            | Op::JumpIfFalseKeep(t)
                if *t <= ip =>
            {
                Some(*t)
            }
            _ => None,
        })
        .collect();
    anchors.sort_unstable();
    anchors.dedup();
    anchors
}

/// The op sequence one iteration of the loop at `anchor` runs: from the header
/// through the backward branch that closes it. `None` when nothing closes it.
fn body_of(ops: &[Op], anchor: usize) -> Option<&[Op]> {
    let close = ops.iter().enumerate().position(|(ip, op)| {
        ip >= anchor
            && matches!(
                op,
                Op::Jump(t) | Op::JumpIfTrue(t) | Op::JumpIfFalse(t)
                    if *t == anchor
            )
    })?;
    Some(&ops[anchor..=close])
}

/// Whether fusevm's block tier accepts this op, asked by handing the JIT a
/// chunk holding just that op. Whole-chunk eligibility is the conjunction of
/// the per-op decision, so a one-op chunk isolates it.
fn op_is_eligible(jit: &JitCompiler, op: &Op) -> bool {
    let mut b = ChunkBuilder::new();
    b.emit(op.clone(), 1);
    jit.is_block_eligible(&b.build())
}

/// An op's variant name, without its operands, so occurrences group.
fn op_name(op: &Op) -> String {
    let text = format!("{op:?}");
    match text.split_once('(') {
        Some((name, _)) => name.to_string(),
        None => text,
    }
}


/// The counted loop this module's tests measure, in the frontend's own syntax.
#[cfg(test)]
const PROGRAM: &str = "f() {\n  local t=0\n  local i=0\n  while (( i < 2000 )); do\n    (( t += i ))\n    (( i += 1 ))\n  done\n}\nf\n";

#[cfg(test)]
mod tests {
    use super::*;

    /// The report can say yes. A counted loop built by hand in the rotated
    /// shape — entered at its body, closed by a conditional backward branch —
    /// is the one shape fusevm's trace compiler accepts (`jit.rs` compiles a
    /// `JumpIfTrue`/`JumpIfFalse` close and bails on an unconditional `Jump`),
    /// and after a run the report finds the installed trace. Without this,
    /// every "not traced" below could be a report that only ever says no.
    #[test]
    fn a_rotated_slot_loop_reaches_a_compiled_trace() {
        let mut b = ChunkBuilder::new();
        b.emit(Op::LoadInt(0), 1);
        b.emit(Op::SetSlot(0), 1);
        let enter = b.emit(Op::Jump(usize::MAX), 1);
        let body = b.current_pos();
        b.emit(Op::GetSlot(0), 1);
        b.emit(Op::LoadInt(1), 1);
        b.emit(Op::Add, 1);
        b.emit(Op::SetSlot(0), 1);
        let cond = b.current_pos();
        b.patch_jump(enter, cond);
        b.emit(Op::GetSlot(0), 1);
        b.emit(Op::LoadInt(200_000), 1);
        b.emit(Op::NumLt, 1);
        b.emit(Op::JumpIfTrue(body), 1);
        b.emit(Op::GetSlot(0), 1);
        let chunk = b.build();

        let mut vm = fusevm::VM::new(chunk.clone());
        vm.enable_tracing_jit();
        vm.run();

        let report = inspect(&chunk);
        assert_eq!(report.chunks[0].loops.len(), 1, "{report}");
        assert!(report.chunks[0].loops[0].traced, "{report}");
        assert!(report.reaches_native(), "{report}");
    }

    /// The top level is not where the loop is — the report has to walk every
    /// chunk to find it. This pins the multi-chunk walk itself: reporting only
    /// `main` would say "no loops" for a program that plainly has one.
    #[test]
    fn the_loop_lives_in_a_chunk_other_than_main() {
        let report = report(PROGRAM).expect("runs");
        assert!(report.chunks.len() > 1, "{report}");
        assert!(report.chunks[0].loops.is_empty(), "main has no loop: {report}");
        assert!(
            report.chunks[1..].iter().any(|c| !c.loops.is_empty()),
            "{report}"
        );
    }

    /// The loop is refused by the tracing tier before branch shape is even
    /// reached: its body dispatches through ops the trace compiler cannot
    /// lower, so `is_trace_eligible` says no. The report names them under
    /// `block-ineligible ops`, which is what makes it a diagnosis rather than
    /// a verdict — this is the list to shrink to reach native code.
    #[test]
    fn the_counted_loop_is_refused_by_the_tracing_tier() {
        let report = report(PROGRAM).expect("runs");
        let looped = report
            .chunks
            .iter()
            .find(|c| !c.loops.is_empty())
            .unwrap_or_else(|| panic!("a chunk with a loop: {report}"));
        assert!(!looped.loops[0].trace_eligible, "{report}");
        assert!(!looped.loops[0].traced, "{report}");
        assert!(!looped.ineligible.is_empty(), "{report}");
        assert!(!report.reaches_native(), "{report}");
    }
}
