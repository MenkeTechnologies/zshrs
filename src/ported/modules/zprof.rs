//! Shell function profiling module - port of Modules/zprof.c
//!
//! Provides zprof builtin for profiling shell functions.

use std::collections::HashMap;
use std::time::Instant;

/// Profile data for a single function.
/// Port of `struct pfunc` from Src/Modules/zprof.c — same fields
/// (name / calls / total / self) the C source's `findpfunc()`
/// (line 97) produces and `cmpsfuncs` / `cmptfuncs` (lines 121/127)
/// sort by.
#[derive(Debug, Clone)]
pub struct ProfFunc {
    pub name: String,
    pub calls: u64,
    pub total_time: f64,
    pub self_time: f64,
    pub num: usize,
}

impl ProfFunc {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zprof.c`.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            calls: 0,
            total_time: 0.0,
            self_time: 0.0,
            num: 0,
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zprof.c`.
    pub fn avg_time(&self) -> f64 {
        if self.calls > 0 {
            self.total_time / self.calls as f64
        } else {
            0.0
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zprof.c`.
    pub fn avg_self(&self) -> f64 {
        if self.calls > 0 {
            self.self_time / self.calls as f64
        } else {
            0.0
        }
    }
}

/// A caller→callee arc with aggregated timing.
/// Port of `struct parc` from Src/Modules/zprof.c — what
/// `findparc()` (line 109) returns and `cmpparcs` (line 133) sorts.
/// Used to render the per-function "callers/callees" section of
/// the report.
#[derive(Debug, Clone)]
pub struct ProfArc {
    pub from: String,
    pub to: String,
    pub calls: u64,
    pub total_time: f64,
    pub self_time: f64,
}

impl ProfArc {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zprof.c`.
    pub fn new(from: &str, to: &str) -> Self {
        Self {
            from: from.to_string(),
            to: to.to_string(),
            calls: 0,
            total_time: 0.0,
            self_time: 0.0,
        }
    }
}

/// Stack frame for tracking function entry timing.
/// Port of the per-call accounting `zprof_wrapper()` from
/// Src/Modules/zprof.c:236 keeps on the C source's call stack —
/// records the function name and entry time so `exit_function`
/// can compute `self_time`.
#[derive(Debug)]
struct StackFrame {
    func_name: String,
    start_time: Instant,
}

/// Profiler state.
/// Port of the file-static `pfuncs` / `parcs` tables Src/Modules/
/// zprof.c keeps to aggregate timing across calls (`zprof_wrapper`
/// at line 236 mutates them; `bin_zprof` at line 139 reads them).
#[derive(Debug, Default)]
pub struct Profiler {
    functions: HashMap<String, ProfFunc>,
    arcs: HashMap<(String, String), ProfArc>,
    stack: Vec<StackFrame>,
    enabled: bool,
}

impl Profiler {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zprof.c`.
    pub fn new() -> Self {
        Self {
            functions: HashMap::new(),
            arcs: HashMap::new(),
            stack: Vec::new(),
            enabled: true,
        }
    }

    /// Begin profiling a function call.
    /// Port of the entry path of `zprof_wrapper()` from
    /// Src/Modules/zprof.c:236 — increments the function's call
    /// count, records the caller→callee arc, and pushes a frame
    /// with the start time.
    pub fn enter_function(&mut self, name: &str) {
        if !self.enabled {
            return;
        }

        let func = self
            .functions
            .entry(name.to_string())
            .or_insert_with(|| ProfFunc::new(name));
        func.calls += 1;

        if let Some(caller) = self.stack.last() {
            let key = (caller.func_name.clone(), name.to_string());
            let arc = self
                .arcs
                .entry(key)
                .or_insert_with(|| ProfArc::new(&caller.func_name, name));
            arc.calls += 1;
        }

        self.stack.push(StackFrame {
            func_name: name.to_string(),
            start_time: Instant::now(),
        });
    }

    /// End profiling a function call.
    /// Port of the exit path of `zprof_wrapper()` from
    /// Src/Modules/zprof.c:236 — pops the matching frame, computes
    /// elapsed time, and updates `self_time` / `total_time` on the
    /// function and the caller arc. Recursion is detected via the
    /// stack walk to avoid double-counting `total_time`.
    pub fn exit_function(&mut self, name: &str) {
        if !self.enabled {
            return;
        }

        if let Some(frame) = self.stack.pop() {
            if frame.func_name != name {
                self.stack.push(frame);
                return;
            }

            let elapsed = frame.start_time.elapsed().as_secs_f64() * 1000.0;

            if let Some(func) = self.functions.get_mut(name) {
                func.self_time += elapsed;

                let is_recursive = self.stack.iter().any(|f| f.func_name == name);
                if !is_recursive {
                    func.total_time += elapsed;
                }
            }

            if let Some(caller) = self.stack.last() {
                let key = (caller.func_name.clone(), name.to_string());
                if let Some(arc) = self.arcs.get_mut(&key) {
                    arc.self_time += elapsed;
                    arc.total_time += elapsed;
                }
            }
        }
    }

    /// Clear all profiling data.
    /// Port of the `-c` (clear) branch inside `bin_zprof()` from
    /// Src/Modules/zprof.c:139 — the C source frees the pfuncs and
    /// parcs lists via `freepfuncs()` (line 74) and `freeparcs()`
    /// (line 86); Rust's `clear()` handles both.
    pub fn clear(&mut self) {
        self.functions.clear();
        self.arcs.clear();
        self.stack.clear();
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zprof.c`.
    /// Enable profiling.
    /// Equivalent to the implicit "loaded" state Src/Modules/zprof.c
    /// is in after `getrandom_buffer()` (line 355) installs `zprof_wrapper`.
    pub fn enable(&mut self) {
        self.enabled = true;
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zprof.c`.
    /// Disable profiling.
    /// Equivalent to the C source's `cleanup_()` (line 367)
    /// detaching the wrapper, but kept resettable via `enable()`.
    pub fn disable(&mut self) {
        self.enabled = false;
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zprof.c`.
    /// Check whether profiling is enabled.
    /// zshrs-original convenience — Src/Modules/zprof.c uses module
    /// load state instead of a flag.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zprof.c`.
    /// Sum self_time across every recorded function.
    /// Used by `report()` to compute the percentage column the C
    /// source emits in `bin_zprof()` (Src/Modules/zprof.c:139).
    pub fn total_time(&self) -> f64 {
        self.functions.values().map(|f| f.self_time).sum()
    }

    /// Get functions sorted by self time, descending.
    /// Port of the `cmpsfuncs()` qsort comparator from
    /// Src/Modules/zprof.c:121 — used for the primary listing
    /// `bin_zprof()` (line 139) prints first.
    pub fn functions_by_self(&self) -> Vec<&ProfFunc> {
        let mut funcs: Vec<_> = self.functions.values().collect();
        funcs.sort_by(|a, b| b.self_time.partial_cmp(&a.self_time).unwrap());
        funcs
    }

    /// Get functions sorted by total time, descending.
    /// Port of the `cmptfuncs()` qsort comparator from
    /// Src/Modules/zprof.c:127 — drives the second pass of
    /// `bin_zprof()` that prints the per-function callers/callees
    /// blocks in total-time order.
    pub fn functions_by_total(&self) -> Vec<&ProfFunc> {
        let mut funcs: Vec<_> = self.functions.values().collect();
        funcs.sort_by(|a, b| b.total_time.partial_cmp(&a.total_time).unwrap());
        funcs
    }

    /// Get arcs sorted by total time, descending.
    /// Port of the `cmpparcs()` comparator from Src/Modules/
    /// zprof.c:133.
    pub fn arcs_by_time(&self) -> Vec<&ProfArc> {
        let mut arcs: Vec<_> = self.arcs.values().collect();
        arcs.sort_by(|a, b| b.total_time.partial_cmp(&a.total_time).unwrap());
        arcs
    }

    /// Render the profile report as a string.
    /// Port of the print path inside `bin_zprof()` from
    /// Src/Modules/zprof.c:139 — emits a self-time-sorted summary
    /// followed by per-function callers/callees blocks. Column
    /// layout matches the C source's `printf` format strings.
    pub fn report(&mut self) -> String {
        let mut output = String::new();
        let total = self.total_time();

        if total == 0.0 {
            return "No profiling data collected.\n".to_string();
        }

        output.push_str(
            "num  calls                time                       self            name\n",
        );
        output.push_str(
            "-----------------------------------------------------------------------------------\n",
        );

        let mut funcs_by_self: Vec<_> = self.functions.values_mut().collect();
        funcs_by_self.sort_by(|a, b| b.self_time.partial_cmp(&a.self_time).unwrap());

        for (i, func) in funcs_by_self.iter_mut().enumerate() {
            func.num = i + 1;
            let time_pct = (func.total_time / total) * 100.0;
            let self_pct = (func.self_time / total) * 100.0;

            output.push_str(&format!(
                "{:2}) {:4}       {:8.2} {:8.2}  {:6.2}%  {:8.2} {:8.2}  {:6.2}%  {}\n",
                func.num,
                func.calls,
                func.total_time,
                func.avg_time(),
                time_pct,
                func.self_time,
                func.avg_self(),
                self_pct,
                func.name
            ));
        }

        let func_nums: HashMap<String, usize> = self
            .functions
            .iter()
            .map(|(name, f)| (name.clone(), f.num))
            .collect();

        let mut funcs_by_total: Vec<_> = self.functions.values().collect();
        funcs_by_total.sort_by(|a, b| b.total_time.partial_cmp(&a.total_time).unwrap());

        for func in funcs_by_total {
            output.push_str("\n-----------------------------------------------------------------------------------\n\n");

            let arcs: Vec<_> = self.arcs.values().filter(|a| a.to == func.name).collect();

            for arc in &arcs {
                let from_num = func_nums.get(&arc.from).copied().unwrap_or(0);
                let time_pct = (arc.total_time / total) * 100.0;
                output.push_str(&format!(
                    "    {:4}/{:<4}  {:8.2} {:8.2}  {:6.2}%  {:8.2} {:8.2}             {} [{}]\n",
                    arc.calls,
                    func.calls,
                    arc.total_time,
                    if arc.calls > 0 {
                        arc.total_time / arc.calls as f64
                    } else {
                        0.0
                    },
                    time_pct,
                    arc.self_time,
                    if arc.calls > 0 {
                        arc.self_time / arc.calls as f64
                    } else {
                        0.0
                    },
                    arc.from,
                    from_num
                ));
            }

            let time_pct = (func.total_time / total) * 100.0;
            let self_pct = (func.self_time / total) * 100.0;
            output.push_str(&format!(
                "{:2}) {:4}       {:8.2} {:8.2}  {:6.2}%  {:8.2} {:8.2}  {:6.2}%  {}\n",
                func.num,
                func.calls,
                func.total_time,
                func.avg_time(),
                time_pct,
                func.self_time,
                func.avg_self(),
                self_pct,
                func.name
            ));

            let callee_arcs: Vec<_> = self.arcs.values().filter(|a| a.from == func.name).collect();

            for arc in callee_arcs.iter().rev() {
                let to_num = func_nums.get(&arc.to).copied().unwrap_or(0);
                let to_calls = self.functions.get(&arc.to).map(|f| f.calls).unwrap_or(0);
                let time_pct = (arc.total_time / total) * 100.0;
                output.push_str(&format!(
                    "    {:4}/{:<4}  {:8.2} {:8.2}  {:6.2}%  {:8.2} {:8.2}             {} [{}]\n",
                    arc.calls,
                    to_calls,
                    arc.total_time,
                    if arc.calls > 0 {
                        arc.total_time / arc.calls as f64
                    } else {
                        0.0
                    },
                    time_pct,
                    arc.self_time,
                    if arc.calls > 0 {
                        arc.self_time / arc.calls as f64
                    } else {
                        0.0
                    },
                    arc.to,
                    to_num
                ));
            }
        }

        output
    }
}

/// `zprof` builtin options.
/// Mirrors the flags `bin_zprof()` from Src/Modules/zprof.c:139
/// parses — currently only `-c` (clear) is supported in the
/// upstream C version; the `compare` and `name` flags exist as
/// future extension hooks.
#[derive(Debug, Default)]
pub struct ZprofOptions {
    pub clear: bool,
}

/// `zprof` builtin entry point.
/// Port of `bin_zprof()` from Src/Modules/zprof.c:139 — `-c`
/// clears the tables, no-arg prints the report.
pub fn bin_zprof(profiler: &mut Profiler, options: &ZprofOptions) -> (i32, String) {
    if options.clear {
        profiler.clear();
        (0, String::new())
    } else {
        (0, profiler.report())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_prof_func_new() {
        let f = ProfFunc::new("test_func");
        assert_eq!(f.name, "test_func");
        assert_eq!(f.calls, 0);
        assert_eq!(f.total_time, 0.0);
        assert_eq!(f.self_time, 0.0);
    }

    #[test]
    fn test_prof_func_avg() {
        let mut f = ProfFunc::new("test");
        f.calls = 4;
        f.total_time = 100.0;
        f.self_time = 80.0;

        assert_eq!(f.avg_time(), 25.0);
        assert_eq!(f.avg_self(), 20.0);
    }

    #[test]
    fn test_prof_arc_new() {
        let a = ProfArc::new("caller", "callee");
        assert_eq!(a.from, "caller");
        assert_eq!(a.to, "callee");
        assert_eq!(a.calls, 0);
    }

    #[test]
    fn test_profiler_new() {
        let p = Profiler::new();
        assert!(p.is_enabled());
        assert!(p.functions.is_empty());
        assert!(p.arcs.is_empty());
    }

    #[test]
    fn test_profiler_enter_exit() {
        let mut p = Profiler::new();

        p.enter_function("func1");
        thread::sleep(Duration::from_millis(10));
        p.exit_function("func1");

        assert_eq!(p.functions.len(), 1);
        let func = p.functions.get("func1").unwrap();
        assert_eq!(func.calls, 1);
        assert!(func.self_time > 0.0);
    }

    #[test]
    fn test_profiler_nested_calls() {
        let mut p = Profiler::new();

        p.enter_function("outer");
        p.enter_function("inner");
        thread::sleep(Duration::from_millis(5));
        p.exit_function("inner");
        p.exit_function("outer");

        assert_eq!(p.functions.len(), 2);
        assert_eq!(p.arcs.len(), 1);

        let arc = p
            .arcs
            .get(&("outer".to_string(), "inner".to_string()))
            .unwrap();
        assert_eq!(arc.calls, 1);
    }

    #[test]
    fn test_profiler_clear() {
        let mut p = Profiler::new();
        p.enter_function("test");
        p.exit_function("test");

        assert!(!p.functions.is_empty());
        p.clear();
        assert!(p.functions.is_empty());
        assert!(p.arcs.is_empty());
    }

    #[test]
    fn test_profiler_disable() {
        let mut p = Profiler::new();
        p.disable();

        p.enter_function("test");
        p.exit_function("test");

        assert!(p.functions.is_empty());
    }

    #[test]
    fn test_builtin_zprof_clear() {
        let mut p = Profiler::new();
        p.enter_function("test");
        p.exit_function("test");

        let options = ZprofOptions { clear: true };
        let (status, _) = bin_zprof(&mut p, &options);

        assert_eq!(status, 0);
        assert!(p.functions.is_empty());
    }

    #[test]
    fn test_builtin_zprof_report() {
        let mut p = Profiler::new();

        let options = ZprofOptions { clear: false };
        let (status, output) = bin_zprof(&mut p, &options);

        assert_eq!(status, 0);
        assert!(output.contains("No profiling data"));
    }
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: module-shims
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::exec::ShellExecutor {
    /// zprof - profiling support
    pub(crate) fn bin_zprof(&mut self, args: &[String]) -> i32 {
        use crate::zprof::ZprofOptions;

        let options = ZprofOptions {
            clear: args.iter().any(|a| a == "-c"),
        };

        let (status, output) = crate::zprof::bin_zprof(&mut self.profiler, &options);
        if !output.is_empty() {
            print!("{}", output);
        }
        status
    }
}
// END moved-from-exec-rs


// ─── moved from src/ported/exec.rs (drift extraction) ───

/// Profiling entry for zprof
#[derive(Clone, Default)]
/// One zprof entry.
/// Port of `struct pfunc` from Src/Modules/zprof.c.
pub struct ProfileEntry {
    pub calls: u64,
    pub total_time_us: u64,
    pub self_time_us: u64,
}

