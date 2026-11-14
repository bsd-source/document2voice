//! Correct text-record decoding for MOBI / PalmDOC books.
//!
//! The `mobi` crate (0.8) trims a fixed `2 × popcount(flags)` bytes from every text
//! record, but MOBI "trailing entries" are variable-length (size stored as a
//! backwards varint at the end of the record), plus an extra 1–4 byte multibyte
//! overlap entry. Getting that wrong leaves junk bytes at every ~4 KB record
//! boundary and mangles headings that sit there.
//!
//! This decodes uncompressed and PalmDOC-compressed books from the raw PDB records.
//! Huffman/CDIC books return `None` and the caller falls back to the crate.

/// Decode the book's text (still HTML markup). `None` if the file is not a
/// PalmDOC/uncompressed MOBI we can read here.
pub(crate) fn decode(bytes: &[u8]) -> Option<String> {
    let records = pdb_records(bytes)?;
    let rec0 = records.first()?;
    if rec0.len() < 16 {
        return None;
    }
    let compression = be16(rec0, 0)?;
    let text_count = usize::from(be16(rec0, 8)?);
    let encryption = be16(rec0, 12)?;
    if encryption != 0 || !matches!(compression, 1 | 2) {
        return None;
    }

    // MOBI header follows the 16-byte PalmDOC header.
    let (utf8, flags) = if rec0.get(16..20) == Some(b"MOBI") {
        let header_len = be32(rec0, 20)? as usize;
        let encoding = be32(rec0, 28)?;
        let flags = if header_len >= 0xE4 {
            be16(rec0, 0xF2).unwrap_or(0)
        } else {
            0
        };
        (encoding == 65001, flags)
    } else {
        (false, 0) // plain PalmDOC: CP1252, no trailing entries
    };

    let mut out = Vec::new();
    for rec in records.iter().skip(1).take(text_count) {
        let data = trim_trailing_entries(rec, flags)?;
        if compression == 2 {
            palmdoc_decompress(data, &mut out);
        } else {
            out.extend_from_slice(data);
        }
    }
    Some(if utf8 {
        String::from_utf8_lossy(&out).into_owned()
    } else {
        out.iter().map(|&b| cp1252(b)).collect()
    })
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// Slices of each record in a Palm database file.
fn pdb_records(bytes: &[u8]) -> Option<Vec<&[u8]>> {
    let n = usize::from(be16(bytes, 76)?);
    let mut offsets = Vec::with_capacity(n);
    for i in 0..n {
        offsets.push(be32(bytes, 78 + i * 8)? as usize);
    }
    let mut recs = Vec::with_capacity(n);
    for i in 0..n {
        let start = offsets[i];
        let end = offsets.get(i + 1).copied().unwrap_or(bytes.len());
        recs.push(bytes.get(start..end.max(start))?);
    }
    Some(recs)
}

/// Strip MOBI trailing entries (see module docs).
fn trim_trailing_entries(mut data: &[u8], flags: u16) -> Option<&[u8]> {
    let trailers = (flags >> 1).count_ones();
    for _ in 0..trailers {
        let tail = &data[data.len().saturating_sub(4)..];
        let mut size = 0usize;
        for &b in tail {
            if b & 0x80 != 0 {
                size = 0;
            }
            size = (size << 7) | usize::from(b & 0x7f);
        }
        data = data.get(..data.len().checked_sub(size)?)?;
    }
    if flags & 1 != 0 {
        let n = usize::from(*data.last()? & 3) + 1;
        data = data.get(..data.len().checked_sub(n)?)?;
    }
    Some(data)
}

/// PalmDOC LZ77 decompression, appending to `out`.
fn palmdoc_decompress(src: &[u8], out: &mut Vec<u8>) {
    let start = out.len();
    let mut i = 0;
    while i < src.len() {
        let c = src[i];
        i += 1;
        match c {
            0x01..=0x08 => {
                let end = (i + usize::from(c)).min(src.len());
                out.extend_from_slice(&src[i..end]);
                i = end;
            }
            0x00 | 0x09..=0x7f => out.push(c),
            0x80..=0xbf => {
                let Some(&next) = src.get(i) else { break };
                i += 1;
                let pair = (u16::from(c) << 8 | u16::from(next)) & 0x3fff;
                let dist = usize::from(pair >> 3);
                let len = usize::from(pair & 7) + 3;
                if dist == 0 || dist > out.len() - start {
                    continue; // corrupt back-reference; skip it
                }
                let from = out.len() - dist;
                for k in 0..len {
                    out.push(out[from + k]);
                }
            }
            0xc0..=0xff => {
                out.push(b' ');
                out.push(c ^ 0x80);
            }
        }
    }
}

/// Windows-1252 byte → char.
pub(crate) fn cp1252(b: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž',
        '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}',
        'ž', 'Ÿ',
    ];
    match b {
        0x80..=0x9f => HIGH[usize::from(b - 0x80)],
        _ => char::from(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trailing_entry_sizes_are_varints() {
        // body "abc", one trailer of 3 bytes (x, y, size=0x83), multibyte entry of 1 byte.
        let rec = [b'a', b'b', b'c', 0x00, b'x', b'y', 0x83];
        assert_eq!(trim_trailing_entries(&rec, 0b11), Some(&b"abc"[..]));
    }

    #[test]
    fn palmdoc_literals_and_backrefs() {
        // "abcabc": literals then back-ref dist=3 len=3 → 0x80 | (3<<3 | 0)
        let src = [b'a', b'b', b'c', 0x80, 0x18];
        let mut out = Vec::new();
        palmdoc_decompress(&src, &mut out);
        assert_eq!(out, b"abcabc");
    }
}
