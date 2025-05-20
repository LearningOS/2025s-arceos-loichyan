use super::{ChunkDecoder, USIZE_BITS};

pub(super) struct Bitmap<'a> {
    bits: &'a mut [usize],
}

impl<'a> Bitmap<'a> {
    pub fn new(bits: &'a mut [usize]) -> Self {
        Self { bits }
    }

    pub fn clear(&mut self) {
        self.bits.fill(0);
    }

    pub fn decode(&self) -> ChunkDecoder {
        ChunkDecoder::new(self.bits)
    }

    pub fn set(&mut self, pos: usize, len: usize) {
        self.apply(pos, len, |b, m| b | m)
    }

    pub fn unset(&mut self, pos: usize, len: usize) {
        self.apply(pos, len, |b, m| b & !m)
    }

    fn apply(&mut self, pos: usize, mut len: usize, mut f: impl FnMut(usize, usize) -> usize) {
        let mut apply = |i: usize, bits| self.bits[i] = f(self.bits[i], bits);

        let mut i = pos / USIZE_BITS;
        let l = pos % USIZE_BITS;
        if l != 0 {
            let n = len.min(USIZE_BITS - l);
            apply(i, !(usize::MAX >> n) >> l);
            len -= n;
            i += 1;
        }

        loop {
            if len > USIZE_BITS {
                len -= USIZE_BITS;
                apply(i, usize::MAX);
                i += 1;
                continue;
            }
            if len > 0 {
                apply(i, !(usize::MAX >> len));
            }
            break;
        }
    }
}

#[test]
fn bitmap_set() {
    let mut bitmap = Bitmap { bits: &mut [0, 0] };

    bitmap.set(5, 10);
    assert_eq!(bitmap.bits, &[0x07fe000000000000, 0x0000000000000000]);

    bitmap.unset(7, 10);
    assert_eq!(bitmap.bits, &[0x0600000000000000, 0x0000000000000000]);

    bitmap.set(56, 10);
    assert_eq!(bitmap.bits, &[0x06000000000000ff, 0xc000000000000000]);

    bitmap.unset(62, 10);
    assert_eq!(bitmap.bits, &[0x06000000000000fc, 0x0000000000000000]);
}
