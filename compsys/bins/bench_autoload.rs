//! Benchmark autoload -Xz: SQLite cache vs filesystem scan

use compsys::cache::{default_cache_path, CompsysCache};
use std::time::Instant;

fn main() {
    let cache_path = default_cache_path();

    if !cache_path.exists() {
        eprintln!("Cache not found at {:?}", cache_path);
        eprintln!("Run compsys-test first to populate the cache");
        std::process::exit(1);
    }

    let cache = CompsysCache::open(&cache_path).unwrap();

    // Get 1000 function names from the cache
    let names: Vec<String> = cache
        .list_autoload_names()
        .unwrap_or_default()
        .into_iter()
        .take(1000)
        .collect();

    if names.is_empty() {
        eprintln!("No autoloads in cache");
        std::process::exit(1);
    }

    println!("Benchmarking {} functions from SQLite cache", names.len());

    // Warm up
    for name in names.iter().take(10) {
        let _ = cache.get_autoload_body(name);
    }

    // Benchmark SQLite lookup
    let start = Instant::now();
    let iterations = 10;
    for _ in 0..iterations {
        for name in &names {
            let _ = cache.get_autoload_body(name);
        }
    }
    let sqlite_elapsed = start.elapsed();
    let total_lookups = names.len() * iterations;
    let avg_ns = sqlite_elapsed.as_nanos() / total_lookups as u128;

    println!("\n=== SQLite Cache (zshrs fast path) ===");
    println!(
        "  Total: {:?} for {} lookups",
        sqlite_elapsed, total_lookups
    );
    println!(
        "  Average: {} ns/lookup ({:.2} µs)",
        avg_ns,
        avg_ns as f64 / 1000.0
    );

    // Benchmark simulated fpath scan (stat + read for each directory)
    let fpath: Vec<std::path::PathBuf> = std::env::var("FPATH")
        .unwrap_or_else(|_| {
            "/usr/local/share/zsh/site-functions:/usr/share/zsh/site-functions:/usr/share/zsh/5.9/functions".to_string()
        })
        .split(':')
        .map(std::path::PathBuf::from)
        .filter(|p| p.exists())
        .collect();

    println!("\n=== Filesystem scan (zsh slow path) ===");
    println!("  fpath has {} directories", fpath.len());

    let start = Instant::now();
    let mut found = 0;
    for name in names.iter().take(100) {
        for dir in &fpath {
            let path = dir.join(name);
            if path.exists() {
                let _ = std::fs::read_to_string(&path);
                found += 1;
                break;
            }
        }
    }
    let fs_elapsed = start.elapsed();
    let fs_avg_ns = if found > 0 {
        fs_elapsed.as_nanos() / found as u128
    } else {
        fs_elapsed.as_nanos() / 100
    };

    println!("  Loaded {} of 100 functions", found);
    println!("  Total: {:?}", fs_elapsed);
    println!(
        "  Average: {} ns/lookup ({:.2} µs)",
        fs_avg_ns,
        fs_avg_ns as f64 / 1000.0
    );

    println!("\n=== Speedup ===");
    let speedup = fs_avg_ns as f64 / avg_ns as f64;
    println!("  SQLite is {:.1}x faster than fpath scan", speedup);
}
