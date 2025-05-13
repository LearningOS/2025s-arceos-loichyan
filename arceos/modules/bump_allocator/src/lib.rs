// The implementation is heavily borrowed from <https://github.com/fitzgen/bumpalo>.

#![no_std]
#![warn(unsafe_op_in_unsafe_fn)]
#![feature(strict_provenance)]

use allocator::{AllocError, AllocResult, BaseAllocator, ByteAllocator, PageAllocator};
use core::alloc::Layout;
use core::iter;
use core::ptr::NonNull;

/// Early memory allocator
/// Use it before formal bytes-allocator and pages-allocator can work!
/// This is a double-end memory range:
/// - Alloc bytes forward
/// - Alloc pages backward
///
/// [ bytes-used | avail-area | pages-used ]
/// |            | -->    <-- |            |
/// start       b_pos        p_pos       end
///
/// For bytes area, 'count' records number of allocations.
/// When it goes down to ZERO, free bytes-used area.
/// For pages area, it will never be freed!
pub struct EarlyAllocator<const PAGE_SIZE: usize> {
    chunks: ChunkFooterList,
    total_bytes: usize,
    used_bytes: usize,
    used_pages: usize,
}

// Safety: No one has the raw pointer except us.
unsafe impl<const PAGE_SIZE: usize> Send for EarlyAllocator<PAGE_SIZE> {}

impl<const PAGE_SIZE: usize> EarlyAllocator<PAGE_SIZE> {
    #[allow(clippy::new_without_default)]
    pub const fn new() -> Self {
        Self {
            chunks: ChunkFooterList(None),
            total_bytes: 0,
            // Allocated bytes, including pages
            used_bytes: 0,
            used_pages: 0,
        }
    }

    unsafe fn add_chunk(&mut self, start: *mut u8, size: usize) -> AllocResult {
        assert!(!start.is_null());

        let end = floor_ptr(
            start.wrapping_add(size - size_of::<ChunkFooter>()),
            align_of::<ChunkFooter>(),
        );
        if end < start {
            return Err(AllocError::NoMemory);
        }

        let footer = end.cast::<ChunkFooter>();
        unsafe { footer.write(ChunkFooter::new(self.chunks, start, end)) };
        self.chunks = ChunkFooterList(NonNull::new(footer));

        self.total_bytes += end.addr() - start.addr();

        Ok(())
    }
}

#[derive(Clone, Copy)]
#[repr(transparent)]
struct ChunkFooterList(Option<NonNull<ChunkFooter>>);

impl ChunkFooterList {
    fn iter_mut(&mut self) -> impl Iterator<Item = &mut ChunkFooter> {
        let mut ptr = self.0;
        iter::from_fn(move || {
            ptr.map(|mut p| {
                let footer = unsafe { p.as_mut() };
                ptr = footer.next.0;
                footer
            })
        })
    }
}

struct ChunkFooter {
    next: ChunkFooterList,
    /// The initial data region is `self.start..self`, namely, we use the
    /// pointer to the footer itself as the end pointer of the entire memory
    /// chunk.
    start: *mut u8,
    /// The end pointer of allocated bytes.
    b_pos: *mut u8,
    /// The start pointer of allocated pages.
    p_pos: *mut u8,
    /// The count of byte allocations alive in this chunk.
    b_count: usize,
}

impl ChunkFooter {
    const fn new(next: ChunkFooterList, start: *mut u8, end: *mut u8) -> Self {
        Self {
            next,
            start,
            b_pos: start,
            p_pos: end,
            b_count: 0,
        }
    }

    fn end(&self) -> *mut u8 {
        self as *const Self as *mut u8
    }
}

impl<const PAGE_SIZE: usize> BaseAllocator for EarlyAllocator<PAGE_SIZE> {
    fn init(&mut self, start: usize, size: usize) {
        self.add_memory(start, size).unwrap();
    }

    fn add_memory(&mut self, start: usize, size: usize) -> AllocResult {
        unsafe { self.add_chunk(start as *mut u8, size) }
    }
}

impl<const PAGE_SIZE: usize> ByteAllocator for EarlyAllocator<PAGE_SIZE> {
    fn alloc(&mut self, layout: Layout) -> AllocResult<NonNull<u8>> {
        for chunk in self.chunks.iter_mut() {
            let start = ceil_ptr(chunk.b_pos, layout.align());
            let end = start.wrapping_add(layout.size());
            if end > chunk.p_pos {
                continue;
            }
            self.used_bytes += end.addr() - chunk.b_pos.addr();
            chunk.b_count += 1;
            chunk.b_pos = end;
            return Ok(NonNull::new(start).unwrap());
        }
        Err(AllocError::NoMemory)
    }

    fn dealloc(&mut self, pos: NonNull<u8>, layout: Layout) {
        let start = pos.as_ptr();
        let end = start.wrapping_add(layout.size());
        for chunk in self.chunks.iter_mut() {
            if start < chunk.start || end > chunk.b_pos {
                continue;
            }
            chunk.b_count -= 1;
            // All bytes are freed, so we can now reset this chunk.
            if chunk.b_count == 0 {
                self.used_bytes -= chunk.b_pos.addr() - chunk.start.addr();
                chunk.b_pos = chunk.start;
            }
            return;
        }
        panic!("invalid address to deallocate: {start:#?}");
    }

    fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    fn used_bytes(&self) -> usize {
        self.used_bytes
    }

    fn available_bytes(&self) -> usize {
        self.total_bytes - self.used_bytes
    }
}

impl<const PAGE_SIZE: usize> PageAllocator for EarlyAllocator<PAGE_SIZE> {
    const PAGE_SIZE: usize = PAGE_SIZE;

    fn alloc_pages(&mut self, num_pages: usize, align_pow2: usize) -> AllocResult<usize> {
        let total_bytes = num_pages * PAGE_SIZE;
        for chunk in self.chunks.iter_mut() {
            let start = floor_ptr(chunk.p_pos.wrapping_sub(total_bytes), align_pow2);
            if start < chunk.p_pos {
                continue;
            }
            self.used_bytes += chunk.p_pos.addr() - start.addr();
            self.used_pages += 1;
            chunk.p_pos = start;
            return Ok(start as usize);
        }
        Err(AllocError::NoMemory)
    }

    fn dealloc_pages(&mut self, pos: usize, num_pages: usize) {
        let start = pos as *mut u8;
        let end = start.wrapping_add(num_pages * PAGE_SIZE);
        for chunk in self.chunks.iter_mut() {
            if start < chunk.p_pos || end > chunk.end() {
                continue;
            }
            self.used_pages -= 1;
            return;
        }
        panic!("invalid address to deallocate: {start:#?}");
    }

    fn total_pages(&self) -> usize {
        self.total_bytes() / PAGE_SIZE
    }

    fn used_pages(&self) -> usize {
        self.used_pages
    }

    fn available_pages(&self) -> usize {
        self.available_bytes() / PAGE_SIZE
    }
}

fn ceil_ptr(ptr: *mut u8, align: usize) -> *mut u8 {
    ptr.with_addr(ceil_addr(ptr.addr(), align))
}

fn floor_ptr(ptr: *mut u8, align: usize) -> *mut u8 {
    ptr.with_addr(floor_addr(ptr.addr(), align))
}

const fn ceil_addr(n: usize, align: usize) -> usize {
    debug_assert!(align > 0);
    debug_assert!(align.is_power_of_two());
    (n + align - 1) & !(align - 1)
}

const fn floor_addr(n: usize, align: usize) -> usize {
    debug_assert!(align > 0);
    debug_assert!(align.is_power_of_two());
    n & !(align - 1)
}

#[cfg(test)]
mod tests {
    #[test]
    fn ceil_addr() {
        assert_eq!(super::ceil_addr(0, 16), 0);
        assert_eq!(super::ceil_addr(1, 16), 16);
        assert_eq!(super::ceil_addr(15, 16), 16);
        assert_eq!(super::ceil_addr(16, 16), 16);
        assert_eq!(super::ceil_addr(17, 16), 32);
    }

    #[test]
    fn floor_addr() {
        assert_eq!(super::floor_addr(0, 16), 0);
        assert_eq!(super::floor_addr(1, 16), 0);
        assert_eq!(super::floor_addr(15, 16), 0);
        assert_eq!(super::floor_addr(16, 16), 16);
        assert_eq!(super::floor_addr(17, 16), 16);
    }
}
