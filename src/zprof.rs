//! Shell function profiling module - port of Modules/zprof.c
//!
//! Provides zprof builtin for profiling shell functions.

use std::collections::HashMap;
use std::time::Instant;

/// Profile data for a single function
#[derive(Debug, Clone)]
pub struct ProfFunc {
    pub name: String,
    pub calls: u64,
    pub total_time: f64,
    pub self_time: f64,
    pub num: usize,
}

impl ProfFunc {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            calls: 0,
            total_time: 0.0,
            self_time: 0.0,
            num: 0,
        }
    }

    pub fn avg_time(&self) -> f64 {
        if self.calls > 0 {
            self.total_time / self.calls as f64
        } else {
            0.0
        }
    }

    pub fn avg_self(&self) -> f64 {
        if self.calls > 0 {
            self.self_time / self.calls as f64
        } else {
            0.0
        }
    }
}

/// Call arc between two functions
#[derive(Debug, Clone)]
pub struct ProfArc {
    pub from: String,
    pub to: String,
    pub calls: u64,
    pub total_time: f64,
    pub self_time: f64,
}

impl ProfArc {
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

/// Stack frame for tracking function calls
#[derive(Debug)]
struct StackFrame {
    func_name: String,
    start_time: Instant,
}

/// Profiler state
#[derive(Debug, Default)]
pub struct Profiler {
    functions: HashMap<String, ProfFunc>,
    arcs: HashMap<(String, String), ProfArc>,
    stack: Vec<StackFrame>,
    enabled: bool,
}

impl Profiler {
    pub fn new() -> Self {
        Self {
            functions: HashMap::new(),
            arcs: HashMap::new(),
            stack: Vec::new(),
            enabled: true,
        }
    }

    /// Start profiling a function call
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

    /// End profiling a function call
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

    /// Clear all profiling data
    pub fn clear(&mut self) {
        self.functions.clear();
        self.arcs.clear();
        self.stack.clear();
    }

    /// Enable profiling
    pub fn enable(&mut self) {
        self.enabled = true;
    }

    /// Disable profiling
    pub fn disable(&mut self) {
        self.enabled = false;
    }

    /// Check if profiling is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Get total time across all functions
    pub fn total_time(&self) -> f64 {
        self.functions.values().map(|f| f.self_time).sum()
    }

    /// Get functions sorted by self time (descending)
    pub fn functions_by_self(&self) -> Vec<&ProfFunc> {
        let mut funcs: Vec<_> = self.functions.values().collect();
        funcs.sort_by(|a, b| b.self_time.partial_cmp(&a.self_time).unwrap());
        funcs
    }

    /// Get functions sorted by total time (descending)
    pub fn functions_by_total(&self) -> Vec<&ProfFunc> {
        let mut funcs: Vec<_> = self.functions.values().collect();
        funcs.sort_by(|a, b| b.total_time.partial_cmp(&a.total_time).unwrap());
        funcs
    }

    /// Get arcs sorted by time (descending)
    pub fn arcs_by_time(&self) -> Vec<&ProfArc> {
        let mut arcs: Vec<_> = self.arcs.values().collect();
        arcs.sort_by(|a, b| b.total_time.partial_cmp(&a.total_time).unwrap());
        arcs
    }

    /// Generate profile report
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

/// Options for zprof builtin
#[derive(Debug, Default)]
pub struct ZprofOptions {
    pub clear: bool,
}

/// Execute zprof builtin
pub fn builtin_zprof(profiler: &mut Profiler, options: &ZprofOptions) -> (i32, String) {
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
        let (status, _) = builtin_zprof(&mut p, &options);

        assert_eq!(status, 0);
        assert!(p.functions.is_empty());
    }

    #[test]
    fn test_builtin_zprof_report() {
        let mut p = Profiler::new();

        let options = ZprofOptions { clear: false };
        let (status, output) = builtin_zprof(&mut p, &options);

        assert_eq!(status, 0);
        assert!(output.contains("No profiling data"));
    }
}
