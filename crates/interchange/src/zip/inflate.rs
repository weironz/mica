//! Raw DEFLATE (RFC 1951) decompressor — in-house, no dependencies.
//!
//! Follows the classic puff.c structure: canonical Huffman decoding with
//! per-length counts, the three block types (stored / fixed / dynamic), and
//! LZ77 back-reference copying. Mirrors the (test-verified) Dart
//! implementation this crate replaces.

#[derive(Debug)]
pub struct InflateError(pub &'static str);

impl std::fmt::Display for InflateError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "inflate: {}", self.0)
  }
}
impl std::error::Error for InflateError {}

pub fn inflate(input: &[u8], size_hint: usize) -> Result<Vec<u8>, InflateError> {
  Inflater {
    input,
    out: Vec::with_capacity(size_hint.max(64)),
    pos: 0,
    bit_buf: 0,
    bit_cnt: 0,
  }
  .run()
}

struct Inflater<'a> {
  input: &'a [u8],
  out: Vec<u8>,
  pos: usize,
  bit_buf: u32,
  bit_cnt: u32,
}

const LEN_BASE: [u16; 29] = [
  3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
  163, 195, 227, 258,
];
const LEN_EXTRA: [u8; 29] = [
  0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
  1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049,
  3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
  0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13,
];
/// Order in which code-length code lengths are stored (RFC 1951 §3.2.7).
const CL_ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

impl Inflater<'_> {
  fn run(mut self) -> Result<Vec<u8>, InflateError> {
    loop {
      let last = self.bits(1)?;
      match self.bits(2)? {
        0 => self.stored()?,
        1 => self.fixed_block()?,
        2 => self.dynamic_block()?,
        _ => return Err(InflateError("invalid block type")),
      }
      if last == 1 {
        return Ok(self.out);
      }
    }
  }

  fn bits(&mut self, need: u32) -> Result<u32, InflateError> {
    let mut val = self.bit_buf;
    while self.bit_cnt < need {
      let byte = *self
        .input
        .get(self.pos)
        .ok_or(InflateError("unexpected end of input"))?;
      val |= u32::from(byte) << self.bit_cnt;
      self.pos += 1;
      self.bit_cnt += 8;
    }
    self.bit_buf = val >> need;
    self.bit_cnt -= need;
    Ok(val & ((1 << need) - 1))
  }

  fn stored(&mut self) -> Result<(), InflateError> {
    self.bit_buf = 0;
    self.bit_cnt = 0; // discard bits up to the next byte boundary
    if self.pos + 4 > self.input.len() {
      return Err(InflateError("truncated stored block"));
    }
    let len = usize::from(self.input[self.pos]) | (usize::from(self.input[self.pos + 1]) << 8);
    self.pos += 4; // LEN + NLEN (complement not verified)
    if self.pos + len > self.input.len() {
      return Err(InflateError("truncated stored block"));
    }
    self.out.extend_from_slice(&self.input[self.pos..self.pos + len]);
    self.pos += len;
    Ok(())
  }

  fn fixed_block(&mut self) -> Result<(), InflateError> {
    let mut lens = [8u8; 288];
    lens[144..256].fill(9);
    lens[256..280].fill(7);
    let len_code = Huffman::new(&lens);
    let dist_code = Huffman::new(&[5u8; 30]);
    self.codes(&len_code, &dist_code)
  }

  fn dynamic_block(&mut self) -> Result<(), InflateError> {
    let nlen = self.bits(5)? as usize + 257;
    let ndist = self.bits(5)? as usize + 1;
    let ncode = self.bits(4)? as usize + 4;
    let mut cl_lens = [0u8; 19];
    for &slot in CL_ORDER.iter().take(ncode) {
      cl_lens[slot] = self.bits(3)? as u8;
    }
    let cl_code = Huffman::new(&cl_lens);

    let total = nlen + ndist;
    let mut lens = vec![0u8; total];
    let mut i = 0;
    while i < total {
      let sym = self.decode(&cl_code)?;
      match sym {
        0..=15 => {
          lens[i] = sym as u8;
          i += 1;
        }
        16..=18 => {
          let (value, repeat) = match sym {
            16 => {
              if i == 0 {
                return Err(InflateError("bad repeat"));
              }
              (lens[i - 1], 3 + self.bits(2)? as usize)
            }
            17 => (0, 3 + self.bits(3)? as usize),
            _ => (0, 11 + self.bits(7)? as usize),
          };
          if i + repeat > total {
            return Err(InflateError("bad repeat"));
          }
          lens[i..i + repeat].fill(value);
          i += repeat;
        }
        _ => return Err(InflateError("bad code length symbol")),
      }
    }
    if lens[256] == 0 {
      return Err(InflateError("missing end-of-block code"));
    }
    let len_code = Huffman::new(&lens[..nlen]);
    let dist_code = Huffman::new(&lens[nlen..]);
    self.codes(&len_code, &dist_code)
  }

  fn codes(&mut self, len_code: &Huffman, dist_code: &Huffman) -> Result<(), InflateError> {
    loop {
      let sym = self.decode(len_code)?;
      match sym {
        0..=255 => self.out.push(sym as u8),
        256 => return Ok(()),
        _ => {
          let li = sym - 257;
          if li >= LEN_BASE.len() {
            return Err(InflateError("bad length code"));
          }
          let len = usize::from(LEN_BASE[li]) + self.bits(u32::from(LEN_EXTRA[li]))? as usize;
          let di = self.decode(dist_code)?;
          if di >= DIST_BASE.len() {
            return Err(InflateError("bad distance code"));
          }
          let dist = usize::from(DIST_BASE[di]) + self.bits(u32::from(DIST_EXTRA[di]))? as usize;
          if dist > self.out.len() {
            return Err(InflateError("distance too far back"));
          }
          let from = self.out.len() - dist;
          for k in 0..len {
            let b = self.out[from + k];
            self.out.push(b);
          }
        }
      }
    }
  }

  fn decode(&mut self, h: &Huffman) -> Result<usize, InflateError> {
    let (mut code, mut first, mut index) = (0usize, 0usize, 0usize);
    for len in 1..=15 {
      code |= self.bits(1)? as usize;
      let count = usize::from(h.count[len]);
      if code < first + count {
        return Ok(h.symbol[index + (code - first)]);
      }
      index += count;
      first = (first + count) << 1;
      code <<= 1;
    }
    Err(InflateError("invalid Huffman code"))
  }
}

/// Canonical Huffman table: per-length symbol counts + symbols sorted by
/// (length, symbol order), as in puff.c.
struct Huffman {
  count: [u16; 16],
  symbol: Vec<usize>,
}

impl Huffman {
  fn new(lengths: &[u8]) -> Self {
    let mut count = [0u16; 16];
    for &l in lengths {
      count[usize::from(l)] += 1;
    }
    count[0] = 0;
    let mut offs = [0usize; 16];
    for len in 1..16 {
      offs[len] = offs[len - 1] + usize::from(count[len - 1]);
    }
    let mut symbol = vec![0usize; lengths.len()];
    for (s, &l) in lengths.iter().enumerate() {
      if l != 0 {
        symbol[offs[usize::from(l)]] = s;
        offs[usize::from(l)] += 1;
      }
    }
    Huffman { count, symbol }
  }
}
