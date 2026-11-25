//! Literal-alternation index for compiled patterns. zshrs-only: it has no C
//! counterpart, so it lives here rather than in `src/ported`.
//!
//! C's `patmatch` tries the alternatives of `(a|b|c)` one after another
//! (Src/pattern.c:3048-3273). An alternative whose operand is a literal
//! (`P_EXACTLY`) and does not match the input fails at its first node, but
//! the walk still visits it, so one test of an N-way alternation costs N.
//! compdump builds exactly that pattern from every defined `_*` function
//! (Completion/compdump:89, `*/(${(~j.|.)…})`) and tests it against every
//! file in `$fpath`: ~46k alternatives times ~49k files on a large fpath,
//! minutes in zsh and in zshrs alike.
//!
//! The index lists, for one alternation, the alternatives whose operand is a
//! literal, grouped by the literal's text, plus every alternative that is not
//! a literal. At a given input position the candidates are the literals that
//! equal the input there (one hash lookup per distinct literal length) and
//! all the non-literals, tried in their original order. Every alternative it
//! leaves out would have failed at its first node, so the first alternative
//! that matches, and with it the match, is the same as C's.
//!
//! Only plain `P_BRANCH` chains are indexed: a `P_WBRANCH` records a visit
//! mark before it recurses (c:3245-3248) and an exclusion runs its own
//! loop, so both keep the ordinary walk. The caller uses the index only when
//! literals compare as raw bytes: no `(#aN)` budget and no case folding.

use crate::ported::pattern::{P_BRANCH, P_EXACTLY, P_WBRANCH};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Byte layout of a compiled instruction (pattern.rs `I_OP` / `I_NEXT` /
/// `I_BODY`): opcode, 4-byte little-endian next offset, then the payload.
const I_OP: usize = 0;
const I_NEXT: usize = 1;
const I_BODY: usize = 5;

/// Below this many alternatives the ordinary walk is as fast as a lookup.
const MIN_ALTERNATIVES: usize = 16;

/// One indexed alternation.
pub struct AltIndex {
    /// Distinct lengths of the literal alternatives.
    lens: Vec<usize>,
    /// Literal text -> (chain position, branch offset) of each alternative
    /// whose operand is that literal, in chain order.
    by_text: HashMap<Vec<u8>, Vec<(u32, usize)>>,
    /// Alternatives whose operand is not a literal: always candidates.
    others: Vec<(u32, usize)>,
}

impl AltIndex {
    /// Walk the chain that starts at `start`. `None` when the chain holds a
    /// `P_WBRANCH` or is too short to be worth indexing.
    fn build(code: &[u8], start: usize) -> Option<AltIndex> {
        let mut index = AltIndex {
            lens: Vec::new(),
            by_text: HashMap::new(),
            others: Vec::new(),
        };
        let mut br = start;
        let mut pos: u32 = 0;
        loop {
            if code.get(br + I_OP) != Some(&P_BRANCH) {
                return None;
            }
            let operand = br + I_BODY;
            if code.get(operand + I_OP) == Some(&P_EXACTLY) {
                let body = operand + I_BODY;
                let len = u32::from_le_bytes(code.get(body..body + 4)?.try_into().ok()?) as usize;
                let text = code.get(body + 4..body + 4 + len)?.to_vec();
                if !index.lens.contains(&len) {
                    index.lens.push(len);
                }
                index.by_text.entry(text).or_default().push((pos, br));
            } else {
                index.others.push((pos, br));
            }
            pos += 1;
            let next =
                u32::from_le_bytes(code.get(br + I_NEXT..br + I_NEXT + 4)?.try_into().ok()?) as usize;
            match code.get(next + I_OP) {
                Some(&P_BRANCH) if next != 0 => br = next,
                Some(&P_WBRANCH) if next != 0 => return None,
                _ => break,
            }
        }
        (pos as usize >= MIN_ALTERNATIVES).then_some(index)
    }

    /// Branch offsets, in chain order, of the alternatives that can match
    /// `input` (the text from the current position to the end).
    pub fn candidates(&self, input: &[u8]) -> Vec<usize> {
        let mut found: Vec<(u32, usize)> = self.others.clone();
        for &len in &self.lens {
            if let Some(hits) = input.get(..len).and_then(|t| self.by_text.get(t)) {
                found.extend_from_slice(hits);
            }
        }
        found.sort_unstable();
        found.into_iter().map(|(_, br)| br).collect()
    }
}

/// The indexes of one compiled pattern, built the first time each
/// alternation is tried and shared by every clone of the pattern.
#[derive(Clone, Default)]
pub struct AltIndexes(Arc<RwLock<HashMap<usize, Option<Arc<AltIndex>>>>>);

impl AltIndexes {
    /// The index of the alternation whose first branch is at `start` in
    /// `code`, or `None` when that alternation is not indexed.
    pub fn get(&self, code: &[u8], start: usize) -> Option<Arc<AltIndex>> {
        if let Some(known) = self.0.read().ok()?.get(&start) {
            return known.clone();
        }
        let built = AltIndex::build(code, start).map(Arc::new);
        self.0.write().ok()?.insert(start, built.clone());
        built
    }
}

