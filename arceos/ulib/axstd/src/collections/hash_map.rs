use alloc::boxed::Box;
use alloc::vec::Vec;
use core::hash::{Hash, Hasher};
use core::mem;

const INITIAL_STATE: u64 = 0xcbf2_9ce4_8422_2325;
const PRIME: u64 = 0x0100_0000_01b3;

pub struct HashMap<K, V> {
    elems: Box<[Option<(K, V)>]>,
    len: usize,
    hasher: FnvHasher,
}

impl<K, V> HashMap<K, V> {
    pub fn new() -> Self {
        Self {
            elems: Vec::new().into_boxed_slice(),
            len: 0,
            hasher: FnvHasher::new(),
        }
    }

    pub fn iter(&self) -> Iter<K, V> {
        Iter {
            inner: self.elems.iter(),
        }
    }
}

impl<K, V> HashMap<K, V>
where
    K: Eq + Hash,
{
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        if self.len == self.elems.len() {
            self.grow();
        }
        self.really_insert(key, value)
    }

    fn grow(&mut self) {
        let cap = self.elems.len();
        // Double the capacity
        let new_cap = (cap | (cap == 0) as usize) << 1;
        let new_elems = core::iter::repeat_with(|| None)
            .take(new_cap)
            .collect::<Vec<_>>()
            .into_boxed_slice();
        // Clear the buffer
        let old_elems = mem::replace(&mut self.elems, new_elems);
        self.len = 0;
        // Relocate all inserted elements
        for (k, v) in Box::into_iter(old_elems).flatten() {
            self.really_insert(k, v);
        }
    }

    fn really_insert(&mut self, key: K, value: V) -> Option<V> {
        let cap = self.elems.len();
        debug_assert!(self.len < cap);
        assert!(cap > 0);

        key.hash(&mut self.hasher);
        let mut i = (self.hasher.finish() as usize) % cap;
        loop {
            match &mut self.elems[i] {
                Some(occupied) if occupied.0 == key => {
                    break Some(mem::replace(&mut occupied.1, value));
                },
                Some(_) => i = (i + 1) % cap,
                vacant => {
                    *vacant = Some((key, value));
                    self.len += 1;
                    break None;
                },
            }
        }
    }
}

impl<K, V> Default for HashMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Iter<'a, K, V> {
    inner: <&'a [Option<(K, V)>] as IntoIterator>::IntoIter,
}

impl<'a, K, V> Iterator for Iter<'a, K, V> {
    type Item = &'a (K, V);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.find_map(Option::as_ref)
    }
}

struct FnvHasher {
    state: u64,
}

impl FnvHasher {
    const fn new() -> Self {
        Self {
            state: INITIAL_STATE,
        }
    }
}

impl Hasher for FnvHasher {
    fn finish(&self) -> u64 {
        self.state
    }

    fn write(&mut self, bytes: &[u8]) {
        self.state = fnv_hash(self.state, bytes);
    }
}

// Credit: <https://github.com/servo/rust-fnv/blob/4e55052a343a4372c191141f29a17ab829cf1dbc/lib.rs>
// License: MIT OR Apache-2.0
const fn fnv_hash(mut hash: u64, bytes: &[u8]) -> u64 {
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u64;
        hash = hash.wrapping_mul(PRIME);
        i += 1;
    }
    hash
}
