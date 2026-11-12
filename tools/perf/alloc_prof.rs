// A temporary allocation profiler for the RAM pass (docs/RAM_PASS.md). NOT part of the build.
// To use it: append this file to src/bin/last_route/main.rs, add `alloc_prof::start();` as the first line of
// `fn main`, build, and run with LR_ALLOC_DUMP=<seconds> RUST_BACKTRACE=1. After that many seconds it prints the
// live heap by call site (blocks under 16 KB are sampled 1 in 16) and exits. Remove it again before committing:
// it makes the game slow and adds memory of its own.
#[cfg(not(test))]
mod alloc_prof {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
    use std::sync::Mutex;
    pub struct Prof;
    thread_local!(static BUSY: Cell<bool> = const { Cell::new(false) });
    static LIVE: Mutex<Option<HashMap<usize, (usize, String)>>> = Mutex::new(None);
    static TICK: AtomicUsize = AtomicUsize::new(0);
    const BIG: usize = 16 * 1024;
    const DULL: &[&str] = &["note", "alloc", "realloc", "alloc_zeroed", "alloc_impl_runtime", "alloc_impl", "allocate", "allocate_zeroed", "realloc_nonnull", "shrink_impl_runtime", "shrink_impl", "grow_impl_runtime", "grow_impl", "shrink", "grow", "shrink_unchecked", "shrink_to_fit", "finish_grow", "grow_amortized", "grow_one", "reserve", "do_reserve_and_handle", "with_capacity", "with_capacity_in", "try_allocate_in", "from_elem", "push", "exchange_malloc", "new", "box_new_uninit", "try_with_capacity_in", "into_boxed_slice", "to_vec", "clone", "from_iter", "collect", "extend", "extend_desugared", "spec_extend", "extend_trusted", "extend_from_slice", "append_elements", "resize", "extend_with", "insert", "entry", "or_insert_with", "reserve_rehash", "resize_inner", "fallible_with_capacity", "new_uninitialized", "spec_from_iter", "spec_from_iter_nested", "from", "into", "to_owned", "to_string", "try_reserve_exact", "reserve_exact", "try_reserve", "push_str", "to_vec_in", "conv_to_vec", "__rust_alloc", "__rust_realloc", "__rust_alloc_zeroed", "do_alloc", "prepare_resize", "reserve_rehash_inner", "with_capacity_zeroed_in", "push_mut", "allocate_for_layout", "allocate_for_slice_in", "{closure#0}", "rustc_entry", "find_or_find_insert_index", "grow_exact", "reserve_entries"];
    fn note(p: *mut u8, size: usize) {
        if size < 256 || p.is_null() { return; }
        let weight = if size < BIG { if TICK.fetch_add(1, Relaxed) % 16 != 0 { return; } 16 } else { 1 };
        if BUSY.with(|b| b.replace(true)) { return; }
        let bt = std::backtrace::Backtrace::force_capture().to_string();
        let site: Vec<&str> = bt.lines().map(|l| l.trim()).filter(|l| !l.starts_with("at ")).map(|l| l.split_once(": ").map(|x| x.1).unwrap_or(l)).filter(|l| { let base = l.split('<').next().unwrap_or(l); let name = base.rsplit("::").next().unwrap_or(base); !DULL.contains(&name) && !l.contains("alloc_prof") && !l.contains("backtrace") }).map(|l| &l[..l.len().min(48)]).take(5).collect();
        let site = site.join(" < ");
        if let Ok(mut g) = LIVE.lock() { g.get_or_insert_with(HashMap::new).insert(p as usize, (size * weight, site)); }
        BUSY.with(|b| b.set(false));
    }
    fn forget(p: *mut u8, size: usize) {
        if size < 256 || BUSY.with(|b| b.replace(true)) { return; }
        if let Ok(mut g) = LIVE.lock() { if let Some(m) = g.as_mut() { m.remove(&(p as usize)); } }
        BUSY.with(|b| b.set(false));
    }
    unsafe impl GlobalAlloc for Prof {
        unsafe fn alloc(&self, l: Layout) -> *mut u8 { let p = System.alloc(l); note(p, l.size()); p }
        unsafe fn dealloc(&self, p: *mut u8, l: Layout) { forget(p, l.size()); System.dealloc(p, l) }
        unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 { forget(p, l.size()); let q = System.realloc(p, l, n); note(q, n); q }
    }
    pub fn start() {
        let Some(secs) = std::env::var("LR_ALLOC_DUMP").ok().and_then(|s| s.parse::<u64>().ok()) else { return };
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(secs));
            BUSY.with(|b| b.set(true));
            let mut by: HashMap<String, (usize, usize)> = HashMap::new();
            if let Ok(g) = LIVE.lock() { if let Some(m) = g.as_ref() { for (s, site) in m.values() { let e = by.entry(site.clone()).or_default(); e.0 += s; e.1 += 1; } } }
            let mut v: Vec<_> = by.into_iter().collect();
            v.sort_by_key(|x| std::cmp::Reverse(x.1 .0));
            eprintln!("ALLOCPROF total {} KB", v.iter().map(|x| x.1 .0).sum::<usize>() / 1024);
            for (site, (bytes, n)) in v.iter().take(40) { eprintln!("ALLOCPROF {:6} KB {:5}x {}", bytes / 1024, n, site); }
            std::process::exit(0);
        });
    }
}
#[cfg(not(test))]
#[global_allocator]
static ALLOC_PROF: alloc_prof::Prof = alloc_prof::Prof;
