//! A pure-Rust decoder for `EXT_meshopt_compression` buffer views (GL-free,
//! `std`-only, builds for wasm32): meshoptimizer's vertex codec (versions 0
//! and 1), index codec (`TRIANGLES`, versions 0 and 1) and index-sequence
//! codec (`INDICES`), and the `OCTAHEDRAL` / `QUATERNION` / `EXPONENTIAL`
//! filters.  A scalar port of meshoptimizer's reference decoders
//! (`vertexcodec.cpp`, `indexcodec.cpp`, `vertexfilter.cpp`), checked against
//! the C library in the tests (native only).
//!
//! Spec: <https://github.com/KhronosGroup/glTF/blob/main/extensions/2.0/Vendor/EXT_meshopt_compression/README.md>

use anyhow::{bail, ensure, Result};

/// A compressed view's `mode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Attributes,
    Triangles,
    Indices,
}

/// A compressed view's `filter`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    None,
    Octahedral,
    Quaternion,
    Exponential,
}

impl Mode {
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "ATTRIBUTES" => Mode::Attributes,
            "TRIANGLES" => Mode::Triangles,
            "INDICES" => Mode::Indices,
            other => bail!("EXT_meshopt_compression: unknown mode {other:?}"),
        })
    }
}

impl Filter {
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "NONE" => Filter::None,
            "OCTAHEDRAL" => Filter::Octahedral,
            "QUATERNION" => Filter::Quaternion,
            "EXPONENTIAL" => Filter::Exponential,
            other => bail!("EXT_meshopt_compression: unknown filter {other:?}"),
        })
    }
}

/// Decode one compressed buffer view: `count` elements of `stride` bytes.
pub fn decode_view(
    src: &[u8],
    count: usize,
    stride: usize,
    mode: Mode,
    filter: Filter,
) -> Result<Vec<u8>> {
    let mut out = match mode {
        Mode::Attributes => decode_vertex_buffer(src, count, stride)?,
        Mode::Triangles => decode_index_buffer(src, count, stride)?,
        Mode::Indices => decode_index_sequence(src, count, stride)?,
    };
    match filter {
        Filter::None => {}
        _ if mode != Mode::Attributes => bail!("EXT_meshopt_compression: filter on {mode:?}"),
        Filter::Octahedral => filter_oct(&mut out, count, stride)?,
        Filter::Quaternion => filter_quat(&mut out, count, stride)?,
        Filter::Exponential => filter_exp(&mut out, count, stride)?,
    }
    Ok(out)
}

// ── vertex codec ───────────────────────────────────────────────────────

const VERTEX_HEADER: u8 = 0xa0;
const BLOCK_SIZE_BYTES: usize = 8192;
const BLOCK_MAX_SIZE: usize = 256;
const GROUP_SIZE: usize = 16;
const GROUP_DECODE_LIMIT: usize = 24;
const TAIL_MIN_V0: usize = 32;
const TAIL_MIN_V1: usize = 24;
const BITS_V0: [u8; 4] = [0, 2, 4, 8];
const BITS_V1: [u8; 5] = [0, 1, 2, 4, 8];

fn block_size(vertex_size: usize) -> usize {
    ((BLOCK_SIZE_BYTES / vertex_size) & !(GROUP_SIZE - 1)).min(BLOCK_MAX_SIZE)
}

/// A bounds-checked cursor over the encoded bytes.
struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Cursor<'_> {
    fn left(&self) -> usize {
        self.data.len() - self.pos
    }
    fn byte(&mut self) -> Result<u8> {
        let b = *self.data.get(self.pos).ok_or_else(|| anyhow::anyhow!("meshopt: truncated"))?;
        self.pos += 1;
        Ok(b)
    }
}

fn decode_bytes_group(c: &mut Cursor<'_>, out: &mut [u8], bits: u8) -> Result<()> {
    match bits {
        0 => out[..GROUP_SIZE].fill(0),
        8 => {
            ensure!(c.left() >= GROUP_SIZE, "meshopt: truncated group");
            out[..GROUP_SIZE].copy_from_slice(&c.data[c.pos..c.pos + GROUP_SIZE]);
            c.pos += GROUP_SIZE;
        }
        1 | 2 | 4 => {
            let b = bits as usize;
            let header_bytes = GROUP_SIZE * b / 8;
            ensure!(c.left() >= header_bytes, "meshopt: truncated group");
            let head = c.pos;
            let mut var = head + header_bytes;
            let max = (1u8 << b) - 1;
            let per_byte = 8 / b;
            for (i, o) in out[..GROUP_SIZE].iter_mut().enumerate() {
                let byte = c.data[head + i / per_byte];
                // 1-bit groups store their 8 values per byte in reverse order
                let slot = if b == 1 { 7 - (i % 8) } else { i % per_byte };
                let enc = if b == 1 {
                    (byte >> (7 - slot)) & 1
                } else {
                    (byte >> (8 - b * (slot + 1))) & max
                };
                if enc == max {
                    *o = *c.data.get(var).ok_or_else(|| anyhow::anyhow!("meshopt: truncated"))?;
                    var += 1;
                } else {
                    *o = enc;
                }
            }
            c.pos = var;
        }
        _ => bail!("meshopt: bad bit width {bits}"),
    }
    Ok(())
}

fn decode_bytes(c: &mut Cursor<'_>, out: &mut [u8], size: usize, bits: &[u8]) -> Result<()> {
    let header_size = (size / GROUP_SIZE).div_ceil(4);
    ensure!(c.left() >= header_size, "meshopt: truncated byte header");
    let header = c.pos;
    c.pos += header_size;
    for i in (0..size).step_by(GROUP_SIZE) {
        ensure!(c.left() >= GROUP_DECODE_LIMIT, "meshopt: truncated byte groups");
        let h = i / GROUP_SIZE;
        let k = (c.data[header + h / 4] >> ((h % 4) * 2)) & 3;
        let w = *bits.get(k as usize).ok_or_else(|| anyhow::anyhow!("meshopt: bad bits"))?;
        decode_bytes_group(c, &mut out[i..i + GROUP_SIZE], w)?;
    }
    Ok(())
}

fn unzigzag8(v: u8) -> u8 {
    (0u8.wrapping_sub(v & 1)) ^ (v >> 1)
}
fn unzigzag16(v: u16) -> u16 {
    (0u16.wrapping_sub(v & 1)) ^ (v >> 1)
}

/// One 4-byte column `k` of a block: the decoded byte planes `buf` (plane
/// `j` at `j * n`) to `vertex_size`-strided output, delta-decoded against
/// `last` (the previous vertex's 4 bytes of this column).
fn decode_deltas(buf: &[u8], out: &mut [u8], n: usize, vs: usize, last: &[u8], channel: u8) {
    match channel & 3 {
        0 => {
            for j in 0..4 {
                let mut p = last[j];
                for i in 0..n {
                    let v = unzigzag8(buf[j * n + i]).wrapping_add(p);
                    out[i * vs + j] = v;
                    p = v;
                }
            }
        }
        1 => {
            for h in 0..2 {
                let mut p = u16::from_le_bytes([last[2 * h], last[2 * h + 1]]);
                for i in 0..n {
                    let raw = buf[2 * h * n + i] as u16 | (buf[(2 * h + 1) * n + i] as u16) << 8;
                    let v = unzigzag16(raw).wrapping_add(p);
                    out[i * vs + 2 * h..i * vs + 2 * h + 2].copy_from_slice(&v.to_le_bytes());
                    p = v;
                }
            }
        }
        _ => {
            let rot = (32 - (channel >> 4) as u32) & 31;
            let mut p = u32::from_le_bytes([last[0], last[1], last[2], last[3]]);
            for i in 0..n {
                let raw = u32::from_le_bytes([buf[i], buf[n + i], buf[2 * n + i], buf[3 * n + i]]);
                let v = raw.rotate_left(rot) ^ p;
                out[i * vs..i * vs + 4].copy_from_slice(&v.to_le_bytes());
                p = v;
            }
        }
    }
}

/// meshoptimizer's `meshopt_decodeVertexBuffer`.
pub fn decode_vertex_buffer(src: &[u8], count: usize, vertex_size: usize) -> Result<Vec<u8>> {
    ensure!(
        vertex_size > 0 && vertex_size <= 256 && vertex_size.is_multiple_of(4),
        "meshopt: vertex size {vertex_size}"
    );
    ensure!(!src.is_empty(), "meshopt: empty vertex buffer");
    ensure!(src[0] & 0xf0 == VERTEX_HEADER, "meshopt: not a vertex buffer");
    let version = src[0] & 0x0f;
    ensure!(version <= 1, "meshopt: vertex codec version {version}");
    let tail_size = vertex_size + if version == 0 { 0 } else { vertex_size / 4 };
    let tail_pad = tail_size.max(if version == 0 { TAIL_MIN_V0 } else { TAIL_MIN_V1 });
    ensure!(src.len() > tail_pad, "meshopt: truncated vertex tail");
    let tail = &src[src.len() - tail_size..];
    let mut last = tail[..vertex_size].to_vec();
    let channels = if version == 0 { None } else { Some(&tail[vertex_size..]) };
    let data_end = src.len() - tail_pad;
    // the decode may read group data up to the tail (never into it past the
    // padding), exactly like the reference
    let mut c = Cursor { data: &src[..src.len()], pos: 1 };
    let mut out = vec![0u8; count * vertex_size];
    let bs = block_size(vertex_size);
    let mut buf = vec![0u8; BLOCK_MAX_SIZE * 4 + GROUP_SIZE];
    let mut transposed = vec![0u8; BLOCK_SIZE_BYTES];
    let mut off = 0;
    while off < count {
        let n = bs.min(count - off);
        let n_aligned = n.div_ceil(GROUP_SIZE) * GROUP_SIZE;
        let control_size = if version == 0 { 0 } else { vertex_size / 4 };
        ensure!(c.left() >= control_size, "meshopt: truncated control");
        let control = c.pos;
        c.pos += control_size;
        for k in (0..vertex_size).step_by(4) {
            let ctrl_byte = if version == 0 { 0 } else { c.data[control + k / 4] };
            for j in 0..4 {
                let ctrl = (ctrl_byte >> (j * 2)) & 3;
                let plane = &mut buf[j * n..];
                match ctrl {
                    3 => {
                        ensure!(c.left() >= n, "meshopt: truncated literal");
                        plane[..n].copy_from_slice(&c.data[c.pos..c.pos + n]);
                        c.pos += n;
                    }
                    2 => plane[..n].fill(0),
                    _ => {
                        let bits: &[u8] =
                            if version == 0 { &BITS_V0 } else { &BITS_V1[ctrl as usize..] };
                        decode_bytes(&mut c, plane, n_aligned, bits)?;
                    }
                }
                ensure!(c.pos <= data_end + tail_pad, "meshopt: overrun");
            }
            let channel = channels.map_or(0, |ch| ch[k / 4]);
            ensure!(channel & 3 != 3, "meshopt: bad channel {channel}");
            decode_deltas(&buf, &mut transposed[k..], n, vertex_size, &last[k..k + 4], channel);
        }
        out[off * vertex_size..(off + n) * vertex_size]
            .copy_from_slice(&transposed[..n * vertex_size]);
        last.copy_from_slice(&transposed[(n - 1) * vertex_size..n * vertex_size]);
        off += n;
    }
    ensure!(c.pos == data_end, "meshopt: vertex data size mismatch");
    Ok(out)
}

// ── index codecs ───────────────────────────────────────────────────────

const INDEX_HEADER: u8 = 0xe0;
const SEQUENCE_HEADER: u8 = 0xd0;

fn vbyte(c: &mut Cursor<'_>) -> Result<u32> {
    let lead = c.byte()?;
    if lead < 128 {
        return Ok(lead as u32);
    }
    let mut result = (lead & 127) as u32;
    let mut shift = 7;
    for _ in 0..4 {
        let g = c.byte()?;
        result |= ((g & 127) as u32) << shift;
        shift += 7;
        if g < 128 {
            break;
        }
    }
    Ok(result)
}

fn decode_index(c: &mut Cursor<'_>, last: u32) -> Result<u32> {
    let v = vbyte(c)?;
    let d = (v >> 1) ^ 0u32.wrapping_sub(v & 1);
    Ok(last.wrapping_add(d))
}

fn write_index(out: &mut Vec<u8>, size: usize, v: u32) {
    if size == 2 {
        out.extend_from_slice(&(v as u16).to_le_bytes());
    } else {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

/// meshoptimizer's `meshopt_decodeIndexBuffer` (`count` indices, a multiple
/// of 3, of `index_size` 2 or 4 bytes).
pub fn decode_index_buffer(src: &[u8], count: usize, index_size: usize) -> Result<Vec<u8>> {
    ensure!(count.is_multiple_of(3), "meshopt: index count {count} not a multiple of 3");
    ensure!(index_size == 2 || index_size == 4, "meshopt: index size {index_size}");
    ensure!(src.len() >= 1 + count / 3 + 16, "meshopt: truncated index buffer");
    ensure!(src[0] & 0xf0 == INDEX_HEADER, "meshopt: not an index buffer");
    let version = src[0] & 0x0f;
    ensure!(version <= 1, "meshopt: index codec version {version}");
    let mut edge = [[u32::MAX; 2]; 16];
    let mut vert = [u32::MAX; 16];
    let (mut eo, mut vo) = (0usize, 0usize);
    let (mut next, mut last) = (0u32, 0u32);
    let fecmax = if version >= 1 { 13 } else { 15 };
    let code_end = 1 + count / 3;
    let safe_end = src.len() - 16;
    let aux = &src[safe_end..];
    let mut c = Cursor { data: &src[..safe_end + 16], pos: code_end };
    let mut out = Vec::with_capacity(count * index_size);
    let push_v = |vert: &mut [u32; 16], vo: &mut usize, v: u32, cond: bool| {
        vert[*vo] = v;
        *vo = (*vo + cond as usize) & 15;
    };
    let push_e = |edge: &mut [[u32; 2]; 16], eo: &mut usize, a: u32, b: u32| {
        edge[*eo] = [a, b];
        *eo = (*eo + 1) & 15;
    };
    for code in &src[1..code_end] {
        let codetri = *code;
        if codetri < 0xf0 {
            let fe = (codetri >> 4) as usize;
            let [a, b] = edge[(eo.wrapping_sub(1 + fe)) & 15];
            let fec = (codetri & 15) as u32;
            let cv;
            if fec < fecmax {
                let cf = vert[(vo.wrapping_sub(1 + fec as usize)) & 15];
                cv = if fec == 0 { next } else { cf };
                next += (fec == 0) as u32;
                push_v(&mut vert, &mut vo, cv, fec == 0);
            } else {
                ensure!(c.pos <= safe_end, "meshopt: truncated index data");
                cv = if fec != 15 {
                    last.wrapping_add((fec * 2).wrapping_sub(27))
                } else {
                    decode_index(&mut c, last)?
                };
                last = cv;
                push_v(&mut vert, &mut vo, cv, true);
            }
            push_e(&mut edge, &mut eo, cv, b);
            push_e(&mut edge, &mut eo, a, cv);
            for v in [a, b, cv] {
                write_index(&mut out, index_size, v);
            }
        } else if codetri < 0xfe {
            let codeaux = aux[(codetri & 15) as usize];
            let feb = (codeaux >> 4) as usize;
            let fec = (codeaux & 15) as usize;
            let a = next;
            next += 1;
            let b = if feb == 0 { next } else { vert[(vo.wrapping_sub(feb)) & 15] };
            next += (feb == 0) as u32;
            let cv = if fec == 0 { next } else { vert[(vo.wrapping_sub(fec)) & 15] };
            next += (fec == 0) as u32;
            for v in [a, b, cv] {
                write_index(&mut out, index_size, v);
            }
            push_v(&mut vert, &mut vo, a, true);
            push_v(&mut vert, &mut vo, b, feb == 0);
            push_v(&mut vert, &mut vo, cv, fec == 0);
            push_e(&mut edge, &mut eo, b, a);
            push_e(&mut edge, &mut eo, cv, b);
            push_e(&mut edge, &mut eo, a, cv);
        } else {
            ensure!(c.pos <= safe_end, "meshopt: truncated index data");
            let codeaux = c.byte()?;
            let fea = if codetri == 0xfe { 0 } else { 15 };
            let feb = (codeaux >> 4) as usize;
            let fec = (codeaux & 15) as usize;
            if codeaux == 0 {
                next = 0;
            }
            let mut a = if fea == 0 {
                next += 1;
                next - 1
            } else {
                0
            };
            let mut b = if feb == 0 {
                next += 1;
                next - 1
            } else {
                vert[(vo.wrapping_sub(feb)) & 15]
            };
            let mut cv = if fec == 0 {
                next += 1;
                next - 1
            } else {
                vert[(vo.wrapping_sub(fec)) & 15]
            };
            if fea == 15 {
                a = decode_index(&mut c, last)?;
                last = a;
            }
            if feb == 15 {
                b = decode_index(&mut c, last)?;
                last = b;
            }
            if fec == 15 {
                cv = decode_index(&mut c, last)?;
                last = cv;
            }
            for v in [a, b, cv] {
                write_index(&mut out, index_size, v);
            }
            push_v(&mut vert, &mut vo, a, true);
            push_v(&mut vert, &mut vo, b, feb == 0 || feb == 15);
            push_v(&mut vert, &mut vo, cv, fec == 0 || fec == 15);
            push_e(&mut edge, &mut eo, b, a);
            push_e(&mut edge, &mut eo, cv, b);
            push_e(&mut edge, &mut eo, a, cv);
        }
    }
    ensure!(c.pos == safe_end, "meshopt: index data size mismatch");
    Ok(out)
}

/// meshoptimizer's `meshopt_decodeIndexSequence`.
pub fn decode_index_sequence(src: &[u8], count: usize, index_size: usize) -> Result<Vec<u8>> {
    ensure!(index_size == 2 || index_size == 4, "meshopt: index size {index_size}");
    ensure!(src.len() >= 1 + count + 4, "meshopt: truncated index sequence");
    ensure!(src[0] & 0xf0 == SEQUENCE_HEADER, "meshopt: not an index sequence");
    ensure!(src[0] & 0x0f <= 1, "meshopt: index sequence version {}", src[0] & 0x0f);
    let safe_end = src.len() - 4;
    let mut c = Cursor { data: src, pos: 1 };
    let mut last = [0u32; 2];
    let mut out = Vec::with_capacity(count * index_size);
    for _ in 0..count {
        ensure!(c.pos < safe_end, "meshopt: truncated index sequence data");
        let mut v = vbyte(&mut c)?;
        let current = (v & 1) as usize;
        v >>= 1;
        let d = (v >> 1) ^ 0u32.wrapping_sub(v & 1);
        let index = last[current].wrapping_add(d);
        last[current] = index;
        write_index(&mut out, index_size, index);
    }
    ensure!(c.pos == safe_end, "meshopt: index sequence size mismatch");
    Ok(out)
}

// ── filters ────────────────────────────────────────────────────────────

/// Round half away from zero (the reference's scalar `int(x + ±0.5)`).
fn round_away(x: f32) -> i32 {
    (x + if x >= 0.0 { 0.5 } else { -0.5 }) as i32
}

fn filter_oct(data: &mut [u8], count: usize, stride: usize) -> Result<()> {
    match stride {
        4 => {
            for i in 0..count {
                let p = &mut data[i * 4..i * 4 + 4];
                let v = oct([p[0] as i8 as f32, p[1] as i8 as f32, p[2] as i8 as f32], 127.0);
                for (k, x) in v.iter().enumerate() {
                    p[k] = *x as i8 as u8;
                }
            }
        }
        8 => {
            for i in 0..count {
                let p = &mut data[i * 8..i * 8 + 8];
                let r = |k: usize| i16::from_le_bytes([p[2 * k], p[2 * k + 1]]) as f32;
                let v = oct([r(0), r(1), r(2)], 32767.0);
                for (k, x) in v.iter().enumerate() {
                    p[2 * k..2 * k + 2].copy_from_slice(&(*x as i16).to_le_bytes());
                }
            }
        }
        _ => bail!("meshopt: OCTAHEDRAL filter on stride {stride}"),
    }
    Ok(())
}

fn oct([x, y, zi]: [f32; 3], max: f32) -> [i32; 3] {
    let (mut x, mut y) = (x, y);
    let z = zi - x.abs() - y.abs();
    let t = if z >= 0.0 { 0.0 } else { z };
    x += if x >= 0.0 { t } else { -t };
    y += if y >= 0.0 { t } else { -t };
    let l = (x * x + y * y + z * z).sqrt();
    let s = max / l;
    [round_away(x * s), round_away(y * s), round_away(z * s)]
}

fn filter_quat(data: &mut [u8], count: usize, stride: usize) -> Result<()> {
    ensure!(stride == 8, "meshopt: QUATERNION filter on stride {stride}");
    let scale = 32767.0 / 2f32.sqrt();
    for i in 0..count {
        let p = &mut data[i * 8..i * 8 + 8];
        let r = |k: usize| i16::from_le_bytes([p[2 * k], p[2 * k + 1]]);
        let (q0, q1, q2, q3) = (r(0), r(1), r(2), r(3));
        let sf = (q3 | 3) as i32;
        let s = sf as f32;
        let (x, y, z) = (q0 as f32, q1 as f32, q2 as f32);
        let ws = s * s;
        let ww = ws * 2.0 - x * x - y * y - z * z;
        let w = if ww >= 0.0 { ww } else { 0.0 }.sqrt();
        let ss = scale / s;
        let (xf, yf, zf) = (round_away(x * ss), round_away(y * ss), round_away(z * ss));
        let wf = (w * ss + 0.5) as i32;
        let qc = (q3 & 3) as usize;
        let mut set =
            |k: usize, v: i32| p[2 * k..2 * k + 2].copy_from_slice(&(v as i16).to_le_bytes());
        set((qc + 1) & 3, xf);
        set((qc + 2) & 3, yf);
        set((qc + 3) & 3, zf);
        set(qc & 3, wf);
    }
    Ok(())
}

fn filter_exp(data: &mut [u8], count: usize, stride: usize) -> Result<()> {
    ensure!(stride.is_multiple_of(4), "meshopt: EXPONENTIAL filter on stride {stride}");
    for w in data[..count * stride].as_chunks_mut::<4>().0 {
        let v = u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
        let m = ((v << 8) as i32) >> 8;
        let e = (v as i32) >> 24;
        let f = f32::from_bits(((e + 127) as u32) << 23) * m as f32;
        w.copy_from_slice(&f.to_bits().to_le_bytes());
    }
    Ok(())
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    /// A small deterministic PRNG (no `rand` dependency).
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u32 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (self.0 >> 33) as u32
        }
        fn f(&mut self) -> f32 {
            self.next() as f32 / (1u64 << 31) as f32 * 2.0 - 1.0
        }
    }

    fn as_bytes<T: Copy>(v: &[T]) -> Vec<u8> {
        let n = std::mem::size_of_val(v);
        // SAFETY: plain-old-data test arrays, read as bytes
        unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, n) }.to_vec()
    }

    /// Smooth-ish vertex streams (a grid with jitter) of several strides, both
    /// codec versions, every level: byte-identical to the C decoder.
    #[test]
    fn vertex_codec_matches_the_reference() {
        let mut rng = Lcg(7);
        for &(count, stride) in
            &[(1usize, 4usize), (17, 8), (300, 12), (1000, 16), (513, 20), (70, 64)]
        {
            let words = stride / 4;
            let mut v: Vec<[u32; 16]> = Vec::new();
            for i in 0..count {
                let mut w = [0u32; 16];
                for (k, x) in w.iter_mut().take(words).enumerate() {
                    *x = match k % 3 {
                        0 => (i as f32 * 0.25 + rng.f() * 0.01).to_bits(),
                        1 => (i as u32 * 7) ^ (rng.next() & 0xff),
                        _ => rng.next() & 0xffff,
                    };
                }
                v.push(w);
            }
            let raw: Vec<u8> = v.iter().flat_map(|w| as_bytes(&w[..words])).collect();
            for version in [0, 1] {
                for level in [0, 2, 3] {
                    let enc = encode(&raw, count, stride, level, version);
                    let mut want = vec![0u8; count * stride];
                    // SAFETY: C decoder into a sized buffer
                    let rc = unsafe {
                        meshopt::ffi::meshopt_decodeVertexBuffer(
                            want.as_mut_ptr().cast(),
                            count,
                            stride,
                            enc.as_ptr(),
                            enc.len(),
                        )
                    };
                    assert_eq!(rc, 0);
                    assert_eq!(want, raw);
                    let got = decode_vertex_buffer(&enc, count, stride).unwrap();
                    assert_eq!(got, raw, "count {count} stride {stride} v{version} L{level}");
                }
            }
        }
    }

    fn encode(raw: &[u8], count: usize, stride: usize, level: i32, version: i32) -> Vec<u8> {
        // SAFETY: bound-sized output buffer
        unsafe {
            let bound = meshopt::ffi::meshopt_encodeVertexBufferBound(count, stride);
            let mut out = vec![0u8; bound];
            let n = meshopt::ffi::meshopt_encodeVertexBufferLevel(
                out.as_mut_ptr(),
                out.len(),
                raw.as_ptr().cast(),
                count,
                stride,
                level,
                version,
            );
            out.truncate(n);
            out
        }
    }

    #[test]
    fn index_codecs_match_the_reference() {
        let mut rng = Lcg(11);
        // a strip-ish mesh with some random triangles and a restart
        let mut idx: Vec<u32> = Vec::new();
        for i in 0..2000u32 {
            idx.extend([i, i + 1, i + 2]);
            if i % 37 == 0 {
                idx.extend([rng.next() % 3000, rng.next() % 3000, rng.next() % 3000]);
            }
        }
        let vcount = 3000;
        // the codec keeps triangle order but may rotate a triangle's corners:
        // compare with the C decoder, and the triangle set with the input
        let enc = meshopt::encode_index_buffer(&idx, vcount).unwrap();
        let reference: Vec<u32> = meshopt::decode_index_buffer(&enc, idx.len()).unwrap();
        let rot = |t: &[u32]| {
            let m = (0..3).min_by_key(|&k| t[k]).unwrap();
            [t[m], t[(m + 1) % 3], t[(m + 2) % 3]]
        };
        for (a, b) in reference.chunks(3).zip(idx.chunks(3)) {
            assert_eq!(rot(a), rot(b), "same triangle, same winding");
        }
        for size in [2, 4] {
            let got = decode_index_buffer(&enc, idx.len(), size).unwrap();
            let want: Vec<u8> = if size == 2 {
                reference.iter().flat_map(|&i| (i as u16).to_le_bytes()).collect()
            } else {
                reference.iter().flat_map(|&i| i.to_le_bytes()).collect()
            };
            assert_eq!(got, want, "triangles, size {size}");
        }
        // version 0
        // SAFETY: global codec version, restored right after
        unsafe { meshopt::ffi::meshopt_encodeIndexVersion(0) };
        let enc0 = meshopt::encode_index_buffer(&idx, vcount).unwrap();
        unsafe { meshopt::ffi::meshopt_encodeIndexVersion(1) };
        let got: Vec<u32> = decode_index_buffer(&enc0, idx.len(), 4)
            .unwrap()
            .chunks(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        let reference0: Vec<u32> = meshopt::decode_index_buffer(&enc0, idx.len()).unwrap();
        assert_eq!(got, reference0, "triangles v0");
        let seq: Vec<u32> = (0..500).map(|_| rng.next() % 4000).collect();
        let enc = meshopt::encode_index_sequence(&seq, 4000).unwrap();
        let got: Vec<u32> = decode_index_sequence(&enc, seq.len(), 4)
            .unwrap()
            .chunks(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        assert_eq!(got, seq, "indices");
    }

    fn c_filter(name: &str, data: &mut [u8], count: usize, stride: usize) {
        // SAFETY: in-place C filters over a sized buffer
        unsafe {
            match name {
                "oct" => {
                    meshopt::ffi::meshopt_decodeFilterOct(data.as_mut_ptr().cast(), count, stride)
                }
                "quat" => {
                    meshopt::ffi::meshopt_decodeFilterQuat(data.as_mut_ptr().cast(), count, stride)
                }
                _ => meshopt::ffi::meshopt_decodeFilterExp(data.as_mut_ptr().cast(), count, stride),
            }
        }
    }

    /// Max per-component difference between two i16 / i8 streams.
    fn max_diff(a: &[u8], b: &[u8], wide: bool) -> i32 {
        if wide {
            a.chunks(2)
                .zip(b.chunks(2))
                .map(|(x, y)| {
                    (i16::from_le_bytes([x[0], x[1]]) as i32
                        - i16::from_le_bytes([y[0], y[1]]) as i32)
                        .abs()
                })
                .max()
                .unwrap_or(0)
        } else {
            a.iter()
                .zip(b)
                .map(|(x, y)| (*x as i8 as i32 - *y as i8 as i32).abs())
                .max()
                .unwrap_or(0)
        }
    }

    #[test]
    fn filters_match_the_reference() {
        let mut rng = Lcg(3);
        let n = 4000;
        let normals: Vec<[f32; 4]> = (0..n)
            .map(|_| {
                let v = [rng.f(), rng.f(), rng.f()];
                let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-3);
                [v[0] / l, v[1] / l, v[2] / l, 0.0]
            })
            .collect();
        // the C reference rounds ties to even in its SIMD path and away from
        // zero in its scalar one: allow that one-unit tie
        for bits in [8, 12, 16] {
            let enc: Vec<[i16; 4]> = meshopt::encode_filter_oct::<i16>(&normals, bits);
            let mut want = as_bytes(&enc);
            let mut got = want.clone();
            c_filter("oct", &mut want, n, 8);
            filter_oct(&mut got, n, 8).unwrap();
            assert!(max_diff(&want, &got, true) <= 1, "oct16 bits {bits}");
        }
        let enc: Vec<[i8; 4]> = meshopt::encode_filter_oct::<i8>(&normals, 8);
        let mut want = as_bytes(&enc);
        let mut got = want.clone();
        c_filter("oct", &mut want, n, 4);
        filter_oct(&mut got, n, 4).unwrap();
        assert!(max_diff(&want, &got, false) <= 1, "oct8");
        let quats: Vec<[f32; 4]> = (0..n)
            .map(|_| {
                let v = [rng.f(), rng.f(), rng.f(), rng.f()];
                let l = (v.iter().map(|x| x * x).sum::<f32>()).sqrt().max(1e-3);
                v.map(|x| x / l)
            })
            .collect();
        for bits in [10, 12, 16] {
            let enc = meshopt::encode_filter_quat(&quats, bits);
            let mut want = as_bytes(&enc);
            let mut got = want.clone();
            c_filter("quat", &mut want, n, 8);
            filter_quat(&mut got, n, 8).unwrap();
            assert!(max_diff(&want, &got, true) <= 1, "quat bits {bits}");
        }
        let vals: Vec<[f32; 3]> =
            (0..n).map(|_| [rng.f() * 100.0, rng.f() * 1e-3, rng.f() * 5e4]).collect();
        for mode in [meshopt::EncodeExpMode::Separate, meshopt::EncodeExpMode::SharedVector] {
            let enc = meshopt::encode_filter_exp(&vals, 15, mode);
            let mut want = as_bytes(&enc);
            let mut got = want.clone();
            c_filter("exp", &mut want, n, 12);
            filter_exp(&mut got, n, 12).unwrap();
            assert_eq!(want, got, "exp {mode:?}");
        }
    }

    #[test]
    fn decode_view_rejects_garbage() {
        assert!(decode_view(&[0u8; 40], 4, 4, Mode::Attributes, Filter::None).is_err());
        assert!(decode_view(&[0xe1; 3], 3, 2, Mode::Triangles, Filter::None).is_err());
        assert!(Mode::parse("STRIPS").is_err() && Filter::parse("COLOR").is_err());
    }
}
