//! Parallel list / file / tree algorithms behind the `puniq`, `pfirst`, `pany`,
//! `psort`, `pmap_chunked`, `par_lines` and `par_walk` builtins.
//!
//! zshrs-original module; the algorithms are ports of strykelang's
//! `par_list.rs` (`puniq_run`, `pany_run`, `pfirst_run`), `par_lines.rs`
//! (`line_aligned_chunks`, `line_count_bytes`, `line_to_perl_string`) and
//! `par_walk.rs` (`collect_paths`), with `String` items in place of
//! `StrykeValue`. Nothing here touches shell state: the builtins in
//! `ext_builtins.rs` own argument parsing and the per-item subprocess.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use rayon::prelude::*;

/// Bucket for `key` among `p` partitions (stryke `partition_bucket`).
#[inline]
fn partition_bucket(key: &str, p: usize) -> usize {
    if p <= 1 {
        return 0;
    }
    let mut h = DefaultHasher::new();
    key.hash(&mut h);
    (h.finish() as usize) % p
}

fn puniq_sequential(list: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::<String>::new();
    let mut out = Vec::new();
    for v in list {
        if seen.insert(v.clone()) {
            out.push(v);
        }
    }
    out
}

/// Hash-partition the list so equal keys land in one bucket, de-duplicate each
/// bucket in parallel keeping the lowest original index, then restore input order.
fn puniq_parallel_buckets(list: Vec<String>, p: usize) -> Vec<String> {
    let mut buckets: Vec<Vec<(usize, String)>> = vec![vec![]; p];
    for (i, v) in list.into_iter().enumerate() {
        buckets[partition_bucket(&v, p)].push((i, v));
    }
    let partials: Vec<Vec<(usize, String)>> = buckets
        .into_par_iter()
        .map(|bucket| {
            let mut seen = HashSet::<String>::new();
            let mut out = Vec::new();
            for (i, v) in bucket {
                if seen.insert(v.clone()) {
                    out.push((i, v));
                }
            }
            out
        })
        .collect();
    let mut merged: Vec<(usize, String)> = partials.into_iter().flatten().collect();
    merged.sort_by_key(|(i, _)| *i);
    merged.into_iter().map(|(_, v)| v).collect()
}

/// Parallel distinct, first-occurrence order (stryke `puniq_run`). Lists shorter
/// than four items per partition take the sequential path.
pub fn puniq_run(list: Vec<String>, num_partitions: usize) -> Vec<String> {
    let n = list.len();
    if n == 0 {
        return vec![];
    }
    let p = num_partitions.max(1);
    if p <= 1 || n < p.saturating_mul(4) {
        puniq_sequential(list)
    } else {
        puniq_parallel_buckets(list, p)
    }
}

/// Short-circuit parallel `any` (stryke `pany_run`): once one test passes the
/// remaining items are skipped without running their test.
pub fn pany_run(list: &[String], test: impl Fn(&str) -> bool + Sync + Send) -> bool {
    let found = AtomicBool::new(false);
    list.par_iter().for_each(|item| {
        if !found.load(Ordering::Relaxed) && test(item) {
            found.store(true, Ordering::Relaxed);
        }
    });
    found.load(Ordering::Relaxed)
}

/// Parallel `first` that keeps the LOWEST list index among passing items
/// (stryke `pfirst_run`); items past the best index found so far are skipped.
pub fn pfirst_run(list: &[String], test: impl Fn(&str) -> bool + Sync + Send) -> Option<usize> {
    if list.is_empty() {
        return None;
    }
    let best = AtomicUsize::new(usize::MAX);
    (0..list.len()).into_par_iter().for_each(|i| {
        if i >= best.load(Ordering::Acquire) {
            return;
        }
        if test(&list[i]) {
            let mut b = best.load(Ordering::Relaxed);
            while i < b {
                match best.compare_exchange_weak(b, i, Ordering::Relaxed, Ordering::Relaxed) {
                    Ok(_) => break,
                    Err(x) => b = x,
                }
            }
        }
    });
    match best.load(Ordering::Relaxed) {
        usize::MAX => None,
        b => Some(b),
    }
}

/// How `psort` compares two items.
#[derive(Copy, Clone, Default)]
pub struct SortSpec {
    /// Compare as numbers (non-numeric items sort as 0, ties fall back to bytes).
    pub numeric: bool,
    /// Descending order.
    pub reverse: bool,
}

/// Parallel stable sort of `list` (stryke's pipeline `psort`; default byte order).
pub fn psort_run(mut list: Vec<String>, spec: SortSpec) -> Vec<String> {
    let num = |s: &str| s.trim().parse::<f64>().unwrap_or(0.0);
    list.par_sort_by(|a, b| {
        let ord = if spec.numeric {
            num(a)
                .partial_cmp(&num(b))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.cmp(b))
        } else {
            a.cmp(b)
        };
        if spec.reverse {
            ord.reverse()
        } else {
            ord
        }
    });
    list
}

/// Split `items` into consecutive chunks of `size` (the last may be shorter).
pub fn chunk_items(items: &[String], size: usize) -> Vec<&[String]> {
    items.chunks(size.max(1)).collect()
}

/// Chunk size that spreads `n` items over about `threads * 8` chunks, the
/// granularity stryke's `par_lines` uses (`num_threads.saturating_mul(8)`).
pub fn auto_chunk_size(n: usize, threads: usize) -> usize {
    let chunks = threads.saturating_mul(8).max(1);
    n.div_ceil(chunks).max(1)
}

/// Byte ranges `[start, end)` covering `data`, each starting at a line boundary
/// (stryke `line_aligned_chunks`), so no line is split across ranges.
pub fn line_aligned_chunks(data: &[u8], max_chunks: usize) -> Vec<(usize, usize)> {
    let len = data.len();
    if len == 0 {
        return vec![];
    }
    let k = max_chunks.max(1).min(len);
    let mut splits: Vec<usize> = (0..=k).map(|i| i * len / k).collect();
    for split in splits.iter_mut().take(k).skip(1) {
        let mut p = *split;
        while p < len && p > 0 && data[p - 1] != b'\n' {
            p += 1;
        }
        *split = p;
    }
    for i in 1..=k {
        if splits[i] < splits[i - 1] {
            splits[i] = splits[i - 1];
        }
    }
    let mut out = Vec::new();
    for i in 0..k {
        let (s, e) = (splits[i], splits[i + 1]);
        if s < e {
            out.push((s, e));
        }
    }
    if out.is_empty() {
        out.push((0, len));
    }
    out
}

/// The lines of one line-aligned byte range, without the `\n` (and without a
/// trailing `\r`, as stryke's `line_to_perl_string` strips CRLF).
pub fn lines_in_range(data: &[u8], start: usize, end: usize) -> Vec<Vec<u8>> {
    let slice = &data[start..end];
    let mut out = Vec::new();
    let mut s = 0usize;
    while s < slice.len() {
        let e = slice[s..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(slice.len(), |p| s + p);
        let mut line = &slice[s..e];
        if line.ends_with(b"\r") {
            line = &line[..line.len() - 1];
        }
        out.push(line.to_vec());
        if e >= slice.len() {
            break;
        }
        s = e + 1;
    }
    out
}

/// Every file and directory under `roots`, each root included when it exists
/// (stryke `par_walk::collect_paths`): entries of one directory are walked in
/// parallel, directory symlinks are followed, other symlinks are visited as files.
pub fn collect_paths(roots: &[PathBuf]) -> Vec<PathBuf> {
    roots.par_iter().flat_map_iter(|r| collect_under(r)).collect()
}

fn collect_under(path: &Path) -> Vec<PathBuf> {
    if path.is_file() || (path.is_symlink() && !path.is_dir()) {
        return vec![path.to_path_buf()];
    }
    if !path.is_dir() {
        return vec![];
    }
    let Ok(read) = std::fs::read_dir(path) else {
        return vec![];
    };
    let mut entries: Vec<_> = read.filter_map(|e| e.ok()).collect();
    // read_dir order is filesystem-dependent; sort so the walk is reproducible.
    entries.sort_by_key(|e| e.file_name());
    let mut out = vec![path.to_path_buf()];
    let sub: Vec<PathBuf> = entries
        .par_iter()
        .flat_map_iter(|e| collect_under(&e.path()))
        .collect();
    out.extend(sub);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn puniq_sequential_keeps_first_occurrence() {
        assert_eq!(puniq_run(s(&["1", "2", "1", "3", "2"]), 1), s(&["1", "2", "3"]));
    }

    #[test]
    fn puniq_parallel_matches_sequential_order() {
        let list = s(&["1", "2", "1", "3", "2", "4", "1", "5"]);
        assert_eq!(puniq_run(list.clone(), 2), puniq_sequential(list));
    }

    #[test]
    fn puniq_parallel_large_input() {
        let list: Vec<String> = (0..5000).map(|i| (i % 97).to_string()).collect();
        assert_eq!(puniq_run(list.clone(), 8), puniq_sequential(list));
    }

    #[test]
    fn pany_finds_and_misses() {
        let l = s(&["1", "2", "3"]);
        assert!(pany_run(&l, |v| v == "2"));
        assert!(!pany_run(&l, |v| v == "4"));
        assert!(!pany_run(&[], |_| true));
    }

    #[test]
    fn pfirst_returns_lowest_passing_index() {
        let l: Vec<String> = (0..200).map(|i| i.to_string()).collect();
        // 40, 41, ... all pass: the lowest index (40) must win whatever the schedule.
        assert_eq!(pfirst_run(&l, |v| v.parse::<i32>().unwrap() >= 40), Some(40));
        assert_eq!(pfirst_run(&l, |_| false), None);
        assert_eq!(pfirst_run(&[], |_| true), None);
    }

    #[test]
    fn psort_byte_numeric_and_reverse() {
        let l = s(&["10", "9", "100", "b", "a"]);
        assert_eq!(psort_run(l.clone(), SortSpec::default()), s(&["10", "100", "9", "a", "b"]));
        assert_eq!(
            psort_run(l.clone(), SortSpec { numeric: true, reverse: false }),
            s(&["a", "b", "9", "10", "100"])
        );
        assert_eq!(
            psort_run(l, SortSpec { numeric: true, reverse: true }),
            s(&["100", "10", "9", "b", "a"])
        );
    }

    #[test]
    fn psort_is_stable_for_equal_numeric_keys() {
        // "1" and "1.0" compare equal numerically; the byte tie-break orders them.
        let out = psort_run(s(&["1.0", "1"]), SortSpec { numeric: true, reverse: false });
        assert_eq!(out, s(&["1", "1.0"]));
    }

    #[test]
    fn chunking_covers_every_item_in_order() {
        let l: Vec<String> = (0..10).map(|i| i.to_string()).collect();
        let chunks = chunk_items(&l, 4);
        assert_eq!(chunks.iter().map(|c| c.len()).collect::<Vec<_>>(), vec![4, 4, 2]);
        let flat: Vec<&String> = chunks.iter().flat_map(|c| c.iter()).collect();
        assert_eq!(flat, l.iter().collect::<Vec<_>>());
        assert_eq!(chunk_items(&l, 0).len(), 10);
    }

    #[test]
    fn auto_chunk_size_is_at_least_one() {
        assert_eq!(auto_chunk_size(0, 4), 1);
        assert_eq!(auto_chunk_size(1000, 4), 32);
        assert_eq!(auto_chunk_size(3, 18), 1);
    }

    #[test]
    fn line_chunks_never_split_a_line() {
        let data = b"a\nbb\nccc\ndddd\n";
        for k in 1..8 {
            let chunks = line_aligned_chunks(data, k);
            let rebuilt: Vec<u8> = chunks.iter().flat_map(|(a, b)| data[*a..*b].iter().copied()).collect();
            assert_eq!(rebuilt, data);
            for (a, _) in &chunks {
                assert!(*a == 0 || data[*a - 1] == b'\n');
            }
        }
        assert!(line_aligned_chunks(b"", 4).is_empty());
    }

    #[test]
    fn lines_in_range_handles_missing_final_newline_and_crlf() {
        let data = b"one\r\ntwo\nthree";
        assert_eq!(
            lines_in_range(data, 0, data.len()),
            vec![b"one".to_vec(), b"two".to_vec(), b"three".to_vec()]
        );
    }

    #[test]
    fn collect_paths_walks_files_and_directories_in_name_order() {
        let base = std::env::temp_dir().join(format!("zshrs_par_ops_walk_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("d")).unwrap();
        std::fs::write(base.join("b.txt"), b"z").unwrap();
        std::fs::write(base.join("d").join("a.txt"), b"z").unwrap();
        let got = collect_paths(std::slice::from_ref(&base));
        let want = vec![
            base.clone(),
            base.join("b.txt"),
            base.join("d"),
            base.join("d").join("a.txt"),
        ];
        assert_eq!(got, want);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn collect_paths_missing_root_is_empty() {
        assert!(collect_paths(&[PathBuf::from("/nonexistent_zshrs_par_ops")]).is_empty());
    }
}
