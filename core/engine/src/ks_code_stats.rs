//! Kitesurf: live counts and bytes of compiled code on this thread, so an embedder can
//! tell what code retains apart from the GC heap and from everything else in memory.

use std::cell::Cell;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Stats {
    pub(crate) blocks: i64,
    pub(crate) blocks_run: i64,
    pub(crate) source_texts: i64,
    pub(crate) source_bytes: i64,
    pub(crate) source_swapped_texts: i64,
    pub(crate) source_swapped_bytes: i64,
    pub(crate) source_reads: i64,
    pub(crate) store_failures: i64,
    pub(crate) packed_bytes: i64,
    pub(crate) expanded_bytes: i64,
    pub(crate) ic_bytes: i64,
    pub(crate) source_map_bytes: i64,
    pub(crate) other_bytes: i64,
}

impl Stats {
    const ZERO: Self = Self {
        blocks: 0,
        blocks_run: 0,
        source_texts: 0,
        source_bytes: 0,
        source_swapped_texts: 0,
        source_swapped_bytes: 0,
        source_reads: 0,
        store_failures: 0,
        packed_bytes: 0,
        expanded_bytes: 0,
        ic_bytes: 0,
        source_map_bytes: 0,
        other_bytes: 0,
    };
}

thread_local! {
    static STATS: Cell<Stats> = const { Cell::new(Stats::ZERO) };
}

#[inline]
pub(crate) fn update(f: impl FnOnce(&mut Stats)) {
    let _ = STATS.try_with(|cell| {
        let mut stats = cell.get();
        f(&mut stats);
        cell.set(stats);
    });
}

#[inline]
pub(crate) fn bytes(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// Kitesurf: live compiled code as (name, value) pairs: blocks, those that ran, resident and
/// swapped source texts, store reads and failures, then bytes by kind. Shared strings uncounted.
#[must_use]
pub fn ks_code_stats() -> [(&'static str, i64); 13] {
    let s = STATS.try_with(Cell::get).unwrap_or(Stats::ZERO);
    [
        ("blocks", s.blocks),
        ("blocksRun", s.blocks_run),
        ("sourceTexts", s.source_texts),
        ("sourceBytes", s.source_bytes),
        ("sourceSwappedTexts", s.source_swapped_texts),
        ("sourceSwappedBytes", s.source_swapped_bytes),
        ("sourceReads", s.source_reads),
        ("storeFailures", s.store_failures),
        ("packedBytes", s.packed_bytes),
        ("expandedBytes", s.expanded_bytes),
        ("icBytes", s.ic_bytes),
        ("sourceMapBytes", s.source_map_bytes),
        ("otherBytes", s.other_bytes),
    ]
}

// What one code block charged to the counters, refunded when it drops. Its inline caches
// are charged later, by `charge_ic`, the first time the block uses one.
#[derive(Debug, Default)]
pub(crate) struct CodeCharge {
    counted: bool,
    fixed: i64,
    map: i64,
    ic: Cell<i64>,
}

impl CodeCharge {
    pub(crate) fn new(fixed: usize, map: usize) -> Self {
        Self::charge(bytes(fixed), bytes(map), 0)
    }

    fn charge(fixed: i64, map: i64, ic: i64) -> Self {
        update(|s| {
            s.blocks += 1;
            s.other_bytes += fixed;
            s.source_map_bytes += map;
            s.ic_bytes += ic;
        });
        Self {
            counted: true,
            fixed,
            map,
            ic: Cell::new(ic),
        }
    }

    pub(crate) fn charge_ic(&self, n: usize) {
        if !self.counted {
            return;
        }
        let n = bytes(n);
        self.ic.set(self.ic.get() + n);
        update(|s| s.ic_bytes += n);
    }
}

impl Clone for CodeCharge {
    fn clone(&self) -> Self {
        if !self.counted {
            return Self::default();
        }
        Self::charge(self.fixed, self.map, self.ic.get())
    }
}

impl Drop for CodeCharge {
    fn drop(&mut self) {
        if !self.counted {
            return;
        }
        let ic = self.ic.get();
        update(|s| {
            s.blocks -= 1;
            s.other_bytes -= self.fixed;
            s.source_map_bytes -= self.map;
            s.ic_bytes -= ic;
        });
    }
}
