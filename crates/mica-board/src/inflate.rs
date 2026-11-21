//! Pure-Rust RFC 1951 (raw DEFLATE) and RFC 1950 (zlib) decompressor.
//! Used for decoding compressed draw.io `<diagram>` payloads and `.drawio.png`
//! `zTXt` chunks without pulling any new third-party crate into `Cargo.lock`.

const MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum InflateError {
  #[error("unexpected end of compressed stream")]
  UnexpectedEof,
  #[error("invalid deflate block header or symbol")]
  InvalidStream,
  #[error("decompressed output exceeded safety limit")]
  OutputTooLarge,
}

/// Inflate a zlib-wrapped stream (RFC 1950: 2-byte header + raw DEFLATE + 4-byte Adler32).
pub fn inflate_zlib(input: &[u8]) -> Result<Vec<u8>, InflateError> {
  if input.len() < 6 {
    return Err(InflateError::UnexpectedEof);
  }
  let cmf = input[0];
  let flg = input[1];
  if (cmf & 0x0F) != 8 || (((cmf as u16) << 8) | (flg as u16)) % 31 != 0 {
    return Err(InflateError::InvalidStream);
  }
  let mut start = 2;
  if (flg & 0x20) != 0 {
    // Preset dictionary (4 bytes)
    start += 4;
    if start > input.len() {
      return Err(InflateError::UnexpectedEof);
    }
  }
  inflate_raw(&input[start..])
}

/// Inflate a raw DEFLATE stream (RFC 1951).
pub fn inflate_raw(input: &[u8]) -> Result<Vec<u8>, InflateError> {
  let mut bits = BitReader::new(input);
  let mut out = Vec::new();

  loop {
    let bfinal = bits.read_bits(1)?;
    let btype = bits.read_bits(2)?;
    match btype {
      0 => {
        bits.align_byte();
        let len = bits.read_u16_le()? as usize;
        let nlen = bits.read_u16_le()? as usize;
        if (len ^ 0xFFFF) != nlen {
          return Err(InflateError::InvalidStream);
        }
        if out.len() + len > MAX_OUTPUT_BYTES {
          return Err(InflateError::OutputTooLarge);
        }
        let slice = bits.read_bytes(len)?;
        out.extend_from_slice(slice);
      }
      1 => {
        let (lit, dist) = fixed_huffman_tables();
        decode_huffman_block(&mut bits, &lit, &dist, &mut out)?;
      }
      2 => {
        let (lit, dist) = read_dynamic_tables(&mut bits)?;
        decode_huffman_block(&mut bits, &lit, &dist, &mut out)?;
      }
      _ => return Err(InflateError::InvalidStream),
    }
    if bfinal != 0 {
      break;
    }
  }
  Ok(out)
}

struct BitReader<'a> {
  data: &'a [u8],
  pos: usize,
  buf: u32,
  nbits: u8,
}

impl<'a> BitReader<'a> {
  fn new(data: &'a [u8]) -> Self {
    Self {
      data,
      pos: 0,
      buf: 0,
      nbits: 0,
    }
  }

  fn ensure(&mut self, n: u8) -> Result<(), InflateError> {
    while self.nbits < n {
      if self.pos >= self.data.len() {
        return Err(InflateError::UnexpectedEof);
      }
      self.buf |= (self.data[self.pos] as u32) << self.nbits;
      self.pos += 1;
      self.nbits += 8;
    }
    Ok(())
  }

  fn read_bits(&mut self, n: u8) -> Result<u32, InflateError> {
    if n == 0 {
      return Ok(0);
    }
    self.ensure(n)?;
    let val = self.buf & ((1u32 << n) - 1);
    self.buf >>= n;
    self.nbits -= n;
    Ok(val)
  }

  fn align_byte(&mut self) {
    let drop = self.nbits & 7;
    self.buf >>= drop;
    self.nbits -= drop;
  }

  fn read_u16_le(&mut self) -> Result<u16, InflateError> {
    let lo = self.read_bits(8)? as u16;
    let hi = self.read_bits(8)? as u16;
    Ok(lo | (hi << 8))
  }

  fn read_bytes(&mut self, len: usize) -> Result<&'a [u8], InflateError> {
    // Rewind unread whole bytes currently sitting in `buf`.
    let buffered_bytes = (self.nbits / 8) as usize;
    self.pos -= buffered_bytes;
    self.buf = 0;
    self.nbits = 0;
    if self.pos + len > self.data.len() {
      return Err(InflateError::UnexpectedEof);
    }
    let slice = &self.data[self.pos..self.pos + len];
    self.pos += len;
    Ok(slice)
  }
}

struct HuffTable {
  counts: [u16; 16],
  symbols: Vec<u16>,
}

impl HuffTable {
  fn build(lengths: &[u8]) -> Result<Self, InflateError> {
    let mut counts = [0u16; 16];
    for &len in lengths {
      if len > 15 {
        return Err(InflateError::InvalidStream);
      }
      if len > 0 {
        counts[len as usize] += 1;
      }
    }
    let mut offsets = [0u16; 16];
    let mut total = 0u16;
    for bits in 1..=15 {
      offsets[bits] = total;
      total = total
        .checked_add(counts[bits])
        .ok_or(InflateError::InvalidStream)?;
    }
    let mut symbols = vec![0u16; total as usize];
    for (sym, &len) in lengths.iter().enumerate() {
      if len > 0 {
        let idx = offsets[len as usize] as usize;
        symbols[idx] = sym as u16;
        offsets[len as usize] += 1;
      }
    }
    Ok(Self { counts, symbols })
  }

  fn decode(&self, bits: &mut BitReader<'_>) -> Result<u16, InflateError> {
    let mut code = 0i32;
    let mut first = 0i32;
    let mut index = 0i32;
    for len in 1..=15 {
      code |= bits.read_bits(1)? as i32;
      let count = self.counts[len] as i32;
      if code - count < first {
        return self
          .symbols
          .get((index + (code - first)) as usize)
          .copied()
          .ok_or(InflateError::InvalidStream);
      }
      index += count;
      first = (first + count) << 1;
      code <<= 1;
    }
    Err(InflateError::InvalidStream)
  }
}

fn fixed_huffman_tables() -> (HuffTable, HuffTable) {
  let mut lit_len = [0u8; 288];
  lit_len[..144].fill(8);
  lit_len[144..256].fill(9);
  lit_len[256..280].fill(7);
  lit_len[280..288].fill(8);
  let dist_len = [5u8; 32];
  (
    HuffTable::build(&lit_len).expect("fixed lit"),
    HuffTable::build(&dist_len).expect("fixed dist"),
  )
}

fn read_dynamic_tables(bits: &mut BitReader<'_>) -> Result<(HuffTable, HuffTable), InflateError> {
  let hlit = bits.read_bits(5)? as usize + 257;
  let hdist = bits.read_bits(5)? as usize + 1;
  let hclen = bits.read_bits(4)? as usize + 4;
  if hlit > 286 || hdist > 30 {
    return Err(InflateError::InvalidStream);
  }
  const ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
  ];
  let mut code_lengths = [0u8; 19];
  for &slot in ORDER.iter().take(hclen) {
    code_lengths[slot] = bits.read_bits(3)? as u8;
  }
  let cl_table = HuffTable::build(&code_lengths)?;
  let total = hlit + hdist;
  let mut lengths = Vec::with_capacity(total);
  while lengths.len() < total {
    let sym = cl_table.decode(bits)?;
    match sym {
      0..=15 => lengths.push(sym as u8),
      16 => {
        let prev = *lengths.last().ok_or(InflateError::InvalidStream)?;
        let rep = bits.read_bits(2)? as usize + 3;
        if lengths.len() + rep > total {
          return Err(InflateError::InvalidStream);
        }
        lengths.resize(lengths.len() + rep, prev);
      }
      17 => {
        let rep = bits.read_bits(3)? as usize + 3;
        if lengths.len() + rep > total {
          return Err(InflateError::InvalidStream);
        }
        lengths.resize(lengths.len() + rep, 0);
      }
      18 => {
        let rep = bits.read_bits(7)? as usize + 11;
        if lengths.len() + rep > total {
          return Err(InflateError::InvalidStream);
        }
        lengths.resize(lengths.len() + rep, 0);
      }
      _ => return Err(InflateError::InvalidStream),
    }
  }
  let lit = HuffTable::build(&lengths[..hlit])?;
  let dist = HuffTable::build(&lengths[hlit..])?;
  Ok((lit, dist))
}

fn decode_huffman_block(
  bits: &mut BitReader<'_>,
  lit: &HuffTable,
  dist: &HuffTable,
  out: &mut Vec<u8>,
) -> Result<(), InflateError> {
  const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
  ];
  const LEN_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
  ];
  const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
  ];
  const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
  ];

  loop {
    let sym = lit.decode(bits)?;
    match sym {
      0..=255 => {
        if out.len() >= MAX_OUTPUT_BYTES {
          return Err(InflateError::OutputTooLarge);
        }
        out.push(sym as u8);
      }
      256 => break,
      257..=285 => {
        let idx = (sym - 257) as usize;
        let length = LEN_BASE[idx] as usize + bits.read_bits(LEN_EXTRA[idx])? as usize;
        let dsym = dist.decode(bits)? as usize;
        if dsym >= 30 {
          return Err(InflateError::InvalidStream);
        }
        let distance = DIST_BASE[dsym] as usize + bits.read_bits(DIST_EXTRA[dsym])? as usize;
        if distance == 0 || distance > out.len() {
          return Err(InflateError::InvalidStream);
        }
        if out.len() + length > MAX_OUTPUT_BYTES {
          return Err(InflateError::OutputTooLarge);
        }
        for _ in 0..length {
          let b = out[out.len() - distance];
          out.push(b);
        }
      }
      _ => return Err(InflateError::InvalidStream),
    }
  }
  Ok(())
}
