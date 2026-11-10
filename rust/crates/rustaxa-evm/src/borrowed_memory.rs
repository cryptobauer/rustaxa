//! Retained native input spans in the pinned Go VM memory pool.
//!
//! This model tracks allocation identity. It does not execute EVM instructions
//! or change gas. The caller supplies the complete logical frame memory after
//! writes and before native reads. Bindings survive frame rollback and release.

use std::{collections::BTreeMap, ops::Range};

const POOL_CAPACITY: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Backing {
    Pool,
    Heap(u64),
}

#[derive(Debug)]
pub(crate) struct FrameMemory {
    offset: usize,
    length: usize,
    backing: Backing,
}

#[derive(Debug)]
struct Binding {
    backing: Backing,
    range: Range<usize>,
    value: Vec<u8>,
}

/// A model belongs to one Go EVM lifetime, including successive trace calls.
#[derive(Debug)]
pub(crate) struct BorrowedMemory<K> {
    reserved: usize,
    next_heap: u64,
    bindings: BTreeMap<K, Binding>,
}

impl<K: Ord> Default for BorrowedMemory<K> {
    fn default() -> Self {
        Self {
            reserved: 0,
            next_heap: 0,
            bindings: BTreeMap::new(),
        }
    }
}

impl<K: Ord> BorrowedMemory<K> {
    pub(crate) fn frame(&self) -> FrameMemory {
        FrameMemory {
            offset: self.reserved,
            length: 0,
            backing: Backing::Pool,
        }
    }

    /// Replaces the raw slot's old binding. Ordinary EVM storage is separate.
    pub(crate) fn remove(&mut self, key: &K) {
        self.bindings.remove(key);
    }

    pub(crate) fn value(&self, key: &K) -> Option<&[u8]> {
        self.bindings
            .get(key)
            .map(|binding| binding.value.as_slice())
    }

    pub(crate) fn values(&self) -> impl Iterator<Item = (&K, &[u8])> {
        self.bindings
            .iter()
            .map(|(key, binding)| (key, binding.value.as_slice()))
    }

    /// Registers a successful native Put that retained a slice of caller memory.
    /// The caller must validate the input span against the completed native Put.
    pub(crate) fn bind(
        &mut self,
        key: K,
        frame: &FrameMemory,
        range: Range<usize>,
        memory: &[u8],
    ) -> bool {
        if memory.len() != frame.length || range.start > range.end || range.end > memory.len() {
            return false;
        }
        let value = memory[range.clone()].to_vec();
        let range = match frame.backing {
            Backing::Pool => frame.offset + range.start..frame.offset + range.end,
            Backing::Heap(_) => range,
        };
        self.bindings.insert(
            key,
            Binding {
                backing: frame.backing,
                range,
                value,
            },
        );
        true
    }

    /// Matches Go Resize followed by the actual memory write. Heap allocations
    /// have exact length; a later heap resize leaves retained old slices intact.
    pub(crate) fn synchronize(&mut self, frame: &mut FrameMemory, memory: &[u8]) {
        assert!(
            memory.len() >= frame.length,
            "logical frame memory cannot shrink"
        );
        if memory.len() > frame.length {
            match frame.backing {
                Backing::Pool if memory.len() <= POOL_CAPACITY.saturating_sub(frame.offset) => {
                    self.reserved = frame.offset + memory.len();
                }
                _ => {
                    if frame.backing == Backing::Pool {
                        self.clear_pool(frame.offset..frame.offset + frame.length);
                        self.reserved = frame.offset;
                    }
                    self.next_heap = self
                        .next_heap
                        .checked_add(1)
                        .expect("bounded VM allocations");
                    frame.backing = Backing::Heap(self.next_heap);
                }
            }
            frame.length = memory.len();
        }
        let offset = if frame.backing == Backing::Pool {
            frame.offset
        } else {
            0
        };
        for binding in self
            .bindings
            .values_mut()
            .filter(|binding| binding.backing == frame.backing)
        {
            let start = binding.range.start.max(offset);
            let end = binding.range.end.min(offset + memory.len());
            if start < end {
                binding.value[start - binding.range.start..end - binding.range.start]
                    .copy_from_slice(&memory[start - offset..end - offset]);
            }
        }
    }

    /// Go clears pooled memory on every exit. Unpooled retained slices survive.
    pub(crate) fn release(&mut self, frame: FrameMemory) {
        if frame.backing == Backing::Pool {
            self.clear_pool(frame.offset..frame.offset + frame.length);
            self.reserved = frame.offset;
        }
    }

    fn clear_pool(&mut self, range: Range<usize>) {
        for binding in self
            .bindings
            .values_mut()
            .filter(|binding| binding.backing == Backing::Pool)
        {
            let start = binding.range.start.max(range.start);
            let end = binding.range.end.min(range.end);
            if start < end {
                binding.value[start - binding.range.start..end - binding.range.start].fill(0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expectations below derive from the pinned Go memory.go, not new Go runs.
    #[test]
    fn pooled_alias_tracks_writes_release_reuse_and_replacement() {
        let mut model = BorrowedMemory::default();
        let mut parent = model.frame();
        model.synchronize(&mut parent, &[0; 32]);
        let mut child = model.frame();
        model.synchronize(&mut child, &[0x77; 96]);
        assert!(model.bind(1, &child, 32..64, &[0x77; 96]));
        assert!(!model.bind(2, &child, 80..100, &[0x77; 96]));
        model.synchronize(&mut child, &[0x55; 96]);
        assert_eq!(model.value(&1), Some([0x55; 32].as_slice()));
        model.release(child);
        assert_eq!(model.value(&1), Some([0; 32].as_slice()));
        let mut reused = model.frame();
        model.synchronize(&mut reused, &[0x33; 64]);
        assert_eq!(model.value(&1), Some([0x33; 32].as_slice()));
        model.remove(&1);
        model.synchronize(&mut reused, &[0x44; 64]);
        assert_eq!(model.value(&1), None);
        assert_eq!(model.values().count(), 0);
        model.release(reused);
        model.release(parent);
        assert_eq!(model.reserved, 0);
    }

    #[test]
    fn pooled_escape_clears_old_span_and_heap_growth_keeps_old_alias() {
        let mut model = BorrowedMemory::default();
        let mut frame = model.frame();
        let mut memory = vec![0x77; 64];
        model.synchronize(&mut frame, &memory);
        assert!(model.bind(1, &frame, 0..32, &memory));
        memory.resize(POOL_CAPACITY + 32, 0);
        model.synchronize(&mut frame, &memory);
        assert_eq!(model.value(&1), Some([0; 32].as_slice()));
        assert!(model.bind(2, &frame, 0..32, &memory));
        memory[0..32].fill(0x55);
        model.synchronize(&mut frame, &memory);
        assert_eq!(model.value(&2), Some([0x55; 32].as_slice()));
        memory.resize(memory.len() + 32, 0);
        memory[0..32].fill(0x33);
        model.synchronize(&mut frame, &memory);
        assert_eq!(model.value(&2), Some([0x55; 32].as_slice()));
        assert!(model.bind(3, &frame, 0..32, &memory));
        model.release(frame);
        assert_eq!(model.value(&3), Some([0x33; 32].as_slice()));
    }

    #[test]
    fn parent_heap_and_nested_pool_have_distinct_allocation_identity() {
        let mut model = BorrowedMemory::default();
        let mut parent = model.frame();
        let memory = vec![0x77; POOL_CAPACITY + 32];
        model.synchronize(&mut parent, &memory);
        assert!(model.bind(1, &parent, 0..32, &memory));
        let mut child = model.frame();
        model.synchronize(&mut child, &[0x55; 64]);
        assert!(model.bind(2, &child, 0..32, &[0x55; 64]));
        model.release(child);
        assert_eq!(model.value(&1), Some([0x77; 32].as_slice()));
        assert_eq!(model.value(&2), Some([0; 32].as_slice()));
        model.release(parent);
        let mut next_call = model.frame();
        model.synchronize(&mut next_call, &[0x33; 32]);
        assert_eq!(model.value(&1), Some([0x77; 32].as_slice()));
        assert_eq!(model.value(&2), Some([0x33; 32].as_slice()));
    }
}
