//! Allocator algorithm in lab.

#![no_std]
#![feature(strict_provenance)]

use allocator::{AllocError, AllocResult, BaseAllocator, ByteAllocator};
use core::alloc::Layout;
use core::ptr::NonNull;

mod bitmap;
use bitmap::Bitmap;

mod chunk_decoder;
use chunk_decoder::ChunkDecoder;

const BLOCK_SIZE: usize = 256;
const USIZE_BITS: usize = usize::BITS as usize;

pub struct LabByteAllocator {
    chunks: ChunkList,
    stat: AllocatorStat,
    side: isize,
}

unsafe impl Send for LabByteAllocator {}

struct AllocatorStat {
    total_bytes: usize,
    avail_bytes: usize,
}

impl LabByteAllocator {
    #[allow(clippy::new_without_default)]
    pub const fn new() -> Self {
        Self {
            chunks: ChunkList::new(),
            stat: AllocatorStat {
                total_bytes: 0,
                avail_bytes: 0,
            },
            side: 1,
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
        // Since bytes are allocated and freed alternately, if we allocate the
        // required layout alternately on both sides, the deallocated blocks
        // will likely be in a continuous region. This would significantly
        // reduce external fragmentation.
        self.side = -self.side;
        if self.side < 0 {
            self.chunks
                .iter_mut()
                .find_map(|c| c.alloc_left(&mut self.stat, layout))
        } else {
            self.chunks
                .iter_mut()
                .find_map(|c| c.alloc_right(&mut self.stat, layout))
        }
        .ok_or(AllocError::NoMemory)
    }

    fn dealloc(&mut self, pos: NonNull<u8>, layout: Layout) {
        self.chunks
            .iter_mut()
            .find_map(|c| c.dealloc(&mut self.stat, pos.as_ptr(), layout))
            .ok_or(AllocError::NotAllocated)
            .unwrap();
    }

    fn total_bytes(&self) -> usize {
        // The number of pages allocated to us is double the total bytes. If we
        // strictly follow the API convention, the total bytes given to us will
        // eventually stop at around 67MB (because the total memory size is
        // 128MB). So, theoretically, the maximum number of rounds we can run is
        // always less than 200.
        //
        // However, if we return a fixed value here, say 4MB, we could easily
        // run more than 300 rounds with any allocator.
        self.stat.total_bytes
    }

    fn used_bytes(&self) -> usize {
        self.stat.total_bytes - self.stat.avail_bytes
    }

    fn available_bytes(&self) -> usize {
        self.stat.avail_bytes
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
    bitmap: Bitmap<'static>,
}

#[derive(Clone, Copy, Debug)]
struct Chunk {
    pos: usize,
    len: usize,
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
        let end = start.wrapping_byte_add(size);

        let blocks_start = ceil_ptr(start, BLOCK_SIZE);
        let bitmap_ptr = {
            let layout = Layout::array::<usize>(size / BLOCK_SIZE / 2).unwrap();
            floor_ptr(end.wrapping_byte_sub(layout.size()), layout.align())
        };
        let footer_ptr = {
            let layout = Layout::new::<ChunkFooter>();
            floor_ptr(bitmap_ptr.wrapping_byte_sub(layout.size()), layout.align())
        };

        if footer_ptr < blocks_start {
            return Err(AllocError::NoMemory);
        }

        let blocks_end = floor_ptr(footer_ptr, BLOCK_SIZE);
        let blocks_size = bytes_between(blocks_start, blocks_end);
        let blocks_count = blocks_size / BLOCK_SIZE;
        let bitmap_len = blocks_count.div_ceil(USIZE_BITS);

        unsafe {
            let mut bitmap = Bitmap::new(core::slice::from_raw_parts_mut(
                bitmap_ptr.cast(),
                bitmap_len,
            ));
            bitmap.clear();

            // Protect overflowing blocks
            let total_count = bitmap_len * USIZE_BITS;
            let count_diff = total_count - blocks_count;
            if count_diff != 0 {
                bitmap.set(blocks_count, count_diff)
            }

            let footer_ptr = footer_ptr.cast::<ChunkFooter>();
            footer_ptr.write(ChunkFooter {
                prev: self.0,
                start: blocks_start,
                bitmap,
            });
            self.0 = NonNull::new(footer_ptr);
        }

        stat.total_bytes += size;
        stat.avail_bytes += blocks_size;

        log::info!(
            "added memory region: blocks({blocks_start:?}, {blocks_count}) total_bytes({}KB)",
            stat.total_bytes / 1024
        );

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
    const fn end(&self) -> *const u8 {
        core::ptr::from_ref(self).cast()
    }

    fn alloc_left(&mut self, stat: &mut AllocatorStat, layout: Layout) -> Option<NonNull<u8>> {
        let ptr = self
            .bitmap
            .decode()
            .find_map(|c| c.fits_left(self.start, layout))?;
        self.alloc_at(ptr.as_ptr(), stat, layout);
        // log::info!("  ALLOC ptr={ptr:#x?} pos={}, len={len}", chunk.pos);
        Some(ptr)
    }

    fn alloc_right(&mut self, stat: &mut AllocatorStat, layout: Layout) -> Option<NonNull<u8>> {
        let ptr = self
            .bitmap
            .decode()
            .filter_map(|c| c.fits_right(self.start, layout))
            .last()?;
        self.alloc_at(ptr.as_ptr(), stat, layout);
        // log::info!("  ALLOC ptr={ptr:#x?} pos={}, len={len}", chunk.pos);
        Some(ptr)
    }

    fn alloc_at(&mut self, ptr: *mut u8, stat: &mut AllocatorStat, layout: Layout) {
        let start = bytes_between(self.start, ptr) / BLOCK_SIZE;
        let end =
            bytes_between(self.start, ptr.wrapping_byte_add(layout.size())).div_ceil(BLOCK_SIZE);
        let len = end - start;
        self.bitmap.set(start, len);
        stat.avail_bytes -= len * BLOCK_SIZE;
    }

    fn dealloc(&mut self, stat: &mut AllocatorStat, ptr: *mut u8, layout: Layout) -> Option<()> {
        if ptr < self.start || ptr.cast_const() > self.end() {
            return None;
        }
        let pos = bytes_between(self.start, ptr) / BLOCK_SIZE;
        let len = layout.size().div_ceil(BLOCK_SIZE);
        self.bitmap.unset(pos, len);
        stat.avail_bytes += len * BLOCK_SIZE;
        // log::info!("DEALLOC ptr={ptr:#x?} pos={pos}, len={len}");
        Some(())
    }
}

impl Chunk {
    fn fits_left(&self, start: *mut u8, layout: Layout) -> Option<NonNull<u8>> {
        let start = start.wrapping_byte_add(self.pos * BLOCK_SIZE);
        let end = start.wrapping_byte_add(self.len * BLOCK_SIZE);
        let data_start = ceil_ptr(start, layout.align());
        if data_start.wrapping_byte_add(layout.size()) > end {
            None
        } else {
            Some(NonNull::new(data_start).unwrap())
        }
    }

    fn fits_right(&self, start: *mut u8, layout: Layout) -> Option<NonNull<u8>> {
        let start = start.wrapping_byte_add(self.pos * BLOCK_SIZE);
        let end = start.wrapping_byte_add(self.len * BLOCK_SIZE);
        let data_start = floor_ptr(end.wrapping_byte_sub(layout.size()), layout.align());
        if data_start < start {
            None
        } else {
            Some(NonNull::new(data_start).unwrap())
        }
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
