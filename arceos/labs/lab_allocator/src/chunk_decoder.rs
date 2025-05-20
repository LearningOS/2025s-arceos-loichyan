use super::Chunk;
use core::slice::Iter as SliceIter;

pub(super) struct ChunkDecoder<'a> {
    iter: SliceIter<'a, usize>,
    peek: EntryLookahead,
    pos: usize,
}

struct EntryLookahead(Option<EntryDecoder>);

#[derive(Clone, Copy)]
struct EntryDecoder {
    bits: usize,
    pos: usize,
}

impl<'a> ChunkDecoder<'a> {
    pub fn new(bits: &'a [usize]) -> Self {
        Self {
            iter: bits.iter(),
            peek: EntryLookahead(None),
            pos: 0,
        }
    }
}

impl EntryLookahead {
    fn pull<'i>(&'i mut self, iter: &mut SliceIter<usize>) -> Option<&'i mut EntryDecoder> {
        let peek = &mut self.0;
        if peek.filter(|p| !p.is_empty()).is_none() {
            *peek = iter
                .next()
                .copied()
                .map(|bits| EntryDecoder { bits, pos: 0 });
        }
        peek.as_mut()
    }
}

impl EntryDecoder {
    const MAX_LEN: usize = usize::BITS as usize;

    fn is_empty(&self) -> bool {
        self.pos == Self::MAX_LEN
    }

    fn skip_ones(&mut self) -> usize {
        let n = self.bits.leading_ones() as usize;
        self.pos += n;
        self.bits = self.bits.checked_shl(n as u32).unwrap_or(0);
        n
    }

    fn skip_zeros(&mut self) -> usize {
        if self.bits == 0 {
            let pos = core::mem::replace(&mut self.pos, Self::MAX_LEN);
            self.pos - pos
        } else {
            let n = self.bits.leading_zeros() as usize;
            self.bits <<= n;
            self.pos += n;
            n
        }
    }
}

impl Iterator for ChunkDecoder<'_> {
    type Item = Chunk;

    fn next(&mut self) -> Option<Self::Item> {
        // Skip leadings ones
        loop {
            let peek = self.peek.pull(&mut self.iter)?;
            self.pos += peek.skip_ones();
            if !peek.is_empty() {
                break;
            }
        }

        // Count leading zeros
        let start = self.pos;
        let mut len = 0;
        loop {
            let Some(peek) = self.peek.pull(&mut self.iter) else {
                break;
            };
            let n = peek.skip_zeros();
            self.pos += n;
            len += n;
            if !peek.is_empty() {
                break;
            }
        }

        if len == 0 {
            None
        } else {
            Some(Chunk { pos: start, len })
        }
    }
}

#[test]
fn test_decoder() {
    #[rustfmt::skip]
    let mut expected = [
        Chunk { pos: 0,   len: 60 },
        Chunk { pos: 61,  len: 1  },
        Chunk { pos: 63,  len: 61 },
        Chunk { pos: 125, len: 1  },
        Chunk { pos: 127, len: 61 },
        Chunk { pos: 189, len: 1  },
        Chunk { pos: 191, len: 1  },
    ]
    .into_iter();
    for (c, e) in ChunkDecoder::new(&[0b1010, 0b1010, 0b1010]).zip(&mut expected) {
        assert_eq!(c.pos, e.pos);
        assert_eq!(c.len, e.len);
    }
    assert_eq!(expected.count(), 0);
}
