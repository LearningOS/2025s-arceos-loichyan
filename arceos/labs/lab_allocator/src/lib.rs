//! Allocator algorithm in lab.

#![no_std]
#![feature(strict_provenance)]

use allocator::{AllocError, AllocResult, BaseAllocator, ByteAllocator};
use core::alloc::Layout;
use core::ptr::NonNull;

pub struct LabByteAllocator {
    chunks: ChunkList,
    stat: AllocatorStat,
    last_pos: isize,
}

unsafe impl Send for LabByteAllocator {}

struct AllocatorStat {
    total_bytes: usize,
    used_bytes: usize,
}

impl LabByteAllocator {
    #[allow(clippy::new_without_default)]
    pub const fn new() -> Self {
        Self {
            chunks: ChunkList::new(),
            stat: AllocatorStat {
                total_bytes: 0,
                used_bytes: 0,
            },
            last_pos: 1,
        }
    }
}

impl BaseAllocator for LabByteAllocator {
    fn init(&mut self, start: usize, size: usize) {
        self.add_memory(start, size).unwrap()
    }

    fn add_memory(&mut self, start: usize, size: usize) -> AllocResult {
        unsafe {
            self.chunks.add(
                &mut self.stat,
                NonNull::new(start as *mut u8).unwrap(),
                size,
            )
        }
    }
}

impl ByteAllocator for LabByteAllocator {
    fn alloc(&mut self, layout: Layout) -> AllocResult<NonNull<u8>> {
        if layout.align() != 1 || !(layout.size() & !0x3ff).is_power_of_two() {
            self.chunks
                .iter_mut()
                .find(|c| c.fits_right(layout).is_some())
                .ok_or(AllocError::NoMemory)?
                .alloc_right(&mut self.stat, layout)
        } else {
            self.last_pos = -self.last_pos;
            if self.last_pos < 0 {
                self.chunks
                    .iter_mut()
                    .find(|c| c.fits_right(layout).is_some())
                    .ok_or(AllocError::NoMemory)?
                    .alloc_right(&mut self.stat, layout)
            } else {
                self.chunks
                    .iter_mut()
                    .filter(|c| c.fits_left(layout).is_some())
                    .last()
                    .ok_or(AllocError::NoMemory)?
                    .alloc_left(&mut self.stat, layout)
            }
        }
        .ok_or_else(|| unreachable!())
    }

    fn dealloc(&mut self, pos: NonNull<u8>, layout: Layout) {
        let ptr = pos.as_ptr();
        self.chunks
            .iter_mut()
            .find(|c| c.contains(ptr, layout))
            .ok_or(AllocError::NotAllocated)
            .unwrap()
            .dealloc(&mut self.stat, ptr, layout)
            .unwrap_or_else(|| unreachable!());
    }

    fn total_bytes(&self) -> usize {
        self.stat.total_bytes
    }

    fn used_bytes(&self) -> usize {
        self.stat.used_bytes
    }

    fn available_bytes(&self) -> usize {
        self.total_bytes() - self.used_bytes()
    }
}

type ChunkPtr = Option<NonNull<ChunkFooter>>;

#[derive(Clone, Copy)]
#[repr(transparent)]
struct ChunkList(ChunkPtr);

#[repr(C)]
struct ChunkFooter {
    prev: ChunkPtr,
    /// The initial data region is `self.start..self`, namely, we use the
    /// pointer to the footer itself as the end pointer of the entire memory
    /// chunk.
    start: *mut u8,
    pos: *mut u8,
    count: usize,
}

impl ChunkList {
    const fn new() -> Self {
        ChunkList(None)
    }

    unsafe fn add(
        &mut self,
        stat: &mut AllocatorStat,
        start: NonNull<u8>,
        size: usize,
    ) -> AllocResult {
        let start = start.as_ptr();

        let end = floor_ptr(
            start.wrapping_byte_add(size - ChunkFooter::SIZE),
            ChunkFooter::ALIGN,
        );
        if end < start {
            return Err(AllocError::NoMemory);
        }

        let chunk_ptr = end.cast::<ChunkFooter>();
        unsafe { chunk_ptr.write(ChunkFooter::new(self.0, start)) };
        self.0 = NonNull::new(chunk_ptr);

        stat.total_bytes += size;
        stat.used_bytes += size - bytes_between(start, end);

        Ok(())
    }

    fn iter_mut(&mut self) -> impl Iterator<Item = &mut ChunkFooter> {
        let mut ptr = self.0;
        core::iter::from_fn(move || {
            ptr.map(|mut p| {
                let chunk = unsafe { p.as_mut() };
                ptr = chunk.prev;
                chunk
            })
        })
    }
}

impl ChunkFooter {
    const ALIGN: usize = align_of::<Self>();
    const SIZE: usize = size_of::<Self>();

    const fn new(prev: ChunkPtr, start: *mut u8) -> Self {
        Self {
            prev,
            start,
            pos: start,
            count: 0,
        }
    }

    const fn end(&self) -> *const u8 {
        core::ptr::from_ref(self).cast()
    }

    fn fits_left(&self, layout: Layout) -> Option<(*mut u8, *mut Self)> {
        let data_ptr = ceil_ptr(self.pos, layout.align());
        let chunk_ptr = ceil_ptr(data_ptr.wrapping_byte_add(layout.size()), Self::ALIGN);
        if chunk_ptr.wrapping_byte_add(Self::SIZE).cast_const() > self.end() {
            None
        } else {
            Some((data_ptr, chunk_ptr.cast()))
        }
    }

    fn alloc_left(&mut self, stat: &mut AllocatorStat, layout: Layout) -> Option<NonNull<u8>> {
        let (data_ptr, chunk_ptr) = self.fits_left(layout)?;
        let data_end = data_ptr.wrapping_byte_add(layout.size());
        let chunk_end = chunk_ptr.wrapping_byte_add(Self::SIZE).cast::<u8>();

        unsafe {
            chunk_ptr.write(Self {
                pos: data_end,
                count: self.count + 1,
                ..Self::new(self.prev, self.start)
            });
        }

        stat.used_bytes += bytes_between(self.pos, chunk_end);
        self.prev = NonNull::new(chunk_ptr);
        self.start = chunk_end;
        self.pos = chunk_end;
        self.count = 0;

        NonNull::new(data_ptr)
    }

    fn fits_right(&self, layout: Layout) -> Option<(*mut Self, *mut u8)> {
        let data_ptr = floor_ptr(
            self.end().wrapping_byte_sub(layout.size()).cast_mut(),
            layout.align(),
        );
        let chunk_ptr = floor_ptr(data_ptr.wrapping_byte_sub(Self::SIZE), Self::ALIGN);
        if chunk_ptr < self.pos {
            None
        } else {
            Some((chunk_ptr.cast(), data_ptr))
        }
    }

    fn alloc_right(&mut self, stat: &mut AllocatorStat, layout: Layout) -> Option<NonNull<u8>> {
        let (chunk_ptr, data_ptr) = self.fits_right(layout)?;
        let data_end = data_ptr.wrapping_byte_add(layout.size());
        let chunk_end = chunk_ptr.wrapping_byte_add(Self::SIZE).cast::<u8>();

        unsafe {
            chunk_ptr.write(core::ptr::from_ref(self).read());
        }

        stat.used_bytes += bytes_between(chunk_ptr, data_end);
        self.prev = NonNull::new(chunk_ptr);
        self.start = chunk_end;
        self.pos = data_end;
        self.count = 1;

        NonNull::new(data_ptr)
    }

    fn contains(&self, ptr: *mut u8, layout: Layout) -> bool {
        ptr >= self.start && ptr.wrapping_byte_add(layout.size()).cast_const() <= self.pos
    }

    fn dealloc(&mut self, stat: &mut AllocatorStat, ptr: *mut u8, layout: Layout) -> Option<()> {
        if !self.contains(ptr, layout) {
            return None;
        }
        self.count -= 1;
        if self.count != 0 {
            return Some(());
        }

        if let Some(prev) = self.prev {
            stat.used_bytes -= bytes_between(prev.as_ptr(), self.pos);
            *self = unsafe { prev.read() };
        } else {
            stat.used_bytes -= bytes_between(self.start, self.pos);
            self.pos = self.start;
        }

        Some(())
    }
}

fn bytes_between<T, U>(start: *const T, end: *const U) -> usize {
    debug_assert!(start.addr() <= end.addr());
    end.addr() - start.addr()
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
