//! The glb extensions [`crate::model::parse_model_glb`] reads that the
//! `gltf` crate rejects: `KHR_mesh_quantization` and
//! `EXT_meshopt_compression`, both in `extensionsRequired` of a classic-roms
//! encoded model.
//!
//! [`normalize_glb`] rewrites such a glb into one the `gltf` crate validates:
//! every `EXT_meshopt_compression` buffer view is decoded
//! ([`crate::meshopt`]) into a plain view of buffer 0 (the fallback buffers
//! are dropped, per the extension's fallback rules), and both extensions leave
//! `extensionsUsed` / `extensionsRequired`.  Quantized accessors stay
//! quantized: the model parser decodes their integer components itself
//! ([`read_vec3`]), and the `gltf` crate's own readers already take
//! normalized integer UVs and rotation keys (core glTF).

use anyhow::{anyhow, bail, ensure, Context, Result};
use serde_json::{json, Value};

use crate::meshopt;

pub const KHR_MESH_QUANTIZATION: &str = "KHR_mesh_quantization";
pub const EXT_MESHOPT_COMPRESSION: &str = "EXT_meshopt_compression";

const CHUNK_JSON: u32 = 0x4E4F_534A;
const CHUNK_BIN: u32 = 0x004E_4942;

fn u32_at(b: &[u8], at: usize) -> Result<u32> {
    let s = b.get(at..at + 4).ok_or_else(|| anyhow!("truncated glb"))?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// Split a glb into its JSON document and BIN chunk.
fn split(bytes: &[u8]) -> Result<(Value, &[u8])> {
    ensure!(bytes.len() >= 20 && &bytes[..4] == b"glTF", "not a glb");
    let (mut at, mut json, mut bin): (usize, Option<Value>, &[u8]) = (12, None, &[]);
    while at + 8 <= bytes.len() {
        let (len, ty) = (u32_at(bytes, at)? as usize, u32_at(bytes, at + 4)?);
        let chunk =
            bytes.get(at + 8..at + 8 + len).ok_or_else(|| anyhow!("truncated glb chunk"))?;
        if ty == CHUNK_JSON && json.is_none() {
            json = Some(serde_json::from_slice(chunk).context("glb JSON chunk")?);
        } else if ty == CHUNK_BIN && bin.is_empty() {
            bin = chunk;
        }
        at += 8 + len;
    }
    Ok((json.ok_or_else(|| anyhow!("glb has no JSON chunk"))?, bin))
}

fn join(json: &Value, bin: &[u8]) -> Result<Vec<u8>> {
    let mut js = serde_json::to_vec(json)?;
    while !js.len().is_multiple_of(4) {
        js.push(b' ');
    }
    let pad = (4 - bin.len() % 4) % 4;
    let total = 12 + 8 + js.len() + if bin.is_empty() { 0 } else { 8 + bin.len() + pad };
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(js.len() as u32).to_le_bytes());
    out.extend_from_slice(&CHUNK_JSON.to_le_bytes());
    out.extend_from_slice(&js);
    if !bin.is_empty() {
        out.extend_from_slice(&((bin.len() + pad) as u32).to_le_bytes());
        out.extend_from_slice(&CHUNK_BIN.to_le_bytes());
        out.extend_from_slice(bin);
        out.extend(std::iter::repeat_n(0u8, pad));
    }
    Ok(out)
}

fn lists(json: &Value, ext: &str) -> bool {
    ["extensionsUsed", "extensionsRequired"]
        .iter()
        .any(|k| json.get(*k).and_then(Value::as_array).is_some_and(|a| a.iter().any(|e| e == ext)))
}

/// A glb the `gltf` crate accepts, or `None` when `bytes` already is one
/// (neither extension used).  Errors on a malformed compressed view, or on
/// an extension in `extensionsRequired` this engine does not know.
pub fn normalize_glb(bytes: &[u8]) -> Result<Option<Vec<u8>>> {
    if bytes.len() < 20 || &bytes[..4] != b"glTF" {
        return Ok(None);
    }
    let (mut json, bin) = split(bytes)?;
    let meshopt = lists(&json, EXT_MESHOPT_COMPRESSION);
    if !meshopt && !lists(&json, KHR_MESH_QUANTIZATION) {
        return Ok(None);
    }
    let bin = if meshopt { decode_views(&mut json, bin)? } else { bin.to_vec() };
    for k in ["extensionsUsed", "extensionsRequired"] {
        let Some(obj) = json.as_object_mut() else { bail!("glb JSON is not an object") };
        if let Some(a) = obj.get_mut(k).and_then(Value::as_array_mut) {
            a.retain(|e| e != KHR_MESH_QUANTIZATION && e != EXT_MESHOPT_COMPRESSION);
            if a.is_empty() {
                obj.remove(k);
            }
        }
    }
    Ok(Some(join(&json, &bin)?))
}

/// Decode every compressed view into a new BIN chunk (plain views copied),
/// re-point every view at buffer 0 and keep only that buffer.
fn decode_views(json: &mut Value, bin: &[u8]) -> Result<Vec<u8>> {
    let buffers = json.get("buffers").and_then(Value::as_array).cloned().unwrap_or_default();
    let fallback = |i: usize| -> bool {
        buffers
            .get(i)
            .and_then(|b| b.pointer("/extensions/EXT_meshopt_compression/fallback"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    };
    let data_of = |i: usize| -> Result<&[u8]> {
        let b = buffers.get(i).ok_or_else(|| anyhow!("no buffer {i}"))?;
        ensure!(b.get("uri").is_none(), "buffer {i}: external uri (a glb reads only its BIN)");
        ensure!(i == 0, "buffer {i} has no data (only the BIN chunk, buffer 0, does)");
        Ok(bin)
    };
    let mut out: Vec<u8> = Vec::new();
    let views = json.get_mut("bufferViews").and_then(Value::as_array_mut);
    for (vi, v) in views.into_iter().flatten().enumerate() {
        let num = |x: &Value, k: &str| x.get(k).and_then(Value::as_u64).map(|n| n as usize);
        let bytes = match v.pointer("/extensions/EXT_meshopt_compression").cloned() {
            Some(ext) => {
                let src = data_of(num(&ext, "buffer").unwrap_or(0))?;
                let off = num(&ext, "byteOffset").unwrap_or(0);
                let len =
                    num(&ext, "byteLength").ok_or_else(|| anyhow!("view {vi}: byteLength"))?;
                let stride =
                    num(&ext, "byteStride").ok_or_else(|| anyhow!("view {vi}: byteStride"))?;
                let count = num(&ext, "count").ok_or_else(|| anyhow!("view {vi}: count"))?;
                let mode =
                    meshopt::Mode::parse(ext.get("mode").and_then(Value::as_str).unwrap_or(""))?;
                let filter = meshopt::Filter::parse(
                    ext.get("filter").and_then(Value::as_str).unwrap_or("NONE"),
                )?;
                let enc = src
                    .get(off..off + len)
                    .ok_or_else(|| anyhow!("view {vi}: compressed range past its buffer"))?;
                let dec = meshopt::decode_view(enc, count, stride, mode, filter)
                    .with_context(|| format!("bufferView {vi}: EXT_meshopt_compression"))?;
                let want = num(v, "byteLength").unwrap_or(dec.len());
                ensure!(dec.len() == want, "view {vi}: decoded {} B, view says {want}", dec.len());
                if let Some(e) = v.get_mut("extensions").and_then(Value::as_object_mut) {
                    e.remove(EXT_MESHOPT_COMPRESSION);
                    if e.is_empty() {
                        v.as_object_mut().map(|o| o.remove("extensions"));
                    }
                }
                dec
            }
            None => {
                let b = num(v, "buffer").unwrap_or(0);
                ensure!(!fallback(b), "view {vi} reads a fallback buffer without compression");
                let src = data_of(b)?;
                let off = num(v, "byteOffset").unwrap_or(0);
                let len = num(v, "byteLength").ok_or_else(|| anyhow!("view {vi}: byteLength"))?;
                src.get(off..off + len)
                    .ok_or_else(|| anyhow!("view {vi}: past its buffer"))?
                    .to_vec()
            }
        };
        while !out.len().is_multiple_of(4) {
            out.push(0);
        }
        v["buffer"] = json!(0);
        v["byteOffset"] = json!(out.len());
        v["byteLength"] = json!(bytes.len());
        out.extend_from_slice(&bytes);
    }
    json["buffers"] = json!([{ "byteLength": out.len() }]);
    Ok(out)
}

/// Read a VEC3 accessor as `f32`s, decoding any component type (normalized
/// integers scaled to `[-1, 1]` / `[0, 1]`, as `KHR_mesh_quantization`
/// stores positions and normals).  The `gltf` crate's position / normal
/// readers assume `FLOAT`.
pub fn read_vec3(
    accessor: &gltf::Accessor<'_>,
    buffers: &[gltf::buffer::Data],
) -> Result<Vec<[f32; 3]>> {
    use gltf::accessor::{DataType, Dimensions};
    ensure!(accessor.dimensions() == Dimensions::Vec3, "accessor {} is not VEC3", accessor.index());
    ensure!(accessor.sparse().is_none(), "accessor {} is sparse", accessor.index());
    let count = accessor.count();
    let Some(view) = accessor.view() else { return Ok(vec![[0.0; 3]; count]) };
    let ty = accessor.data_type();
    let size = ty.size();
    let stride = view.stride().unwrap_or(3 * size);
    let data = &buffers[view.buffer().index()];
    let base = view.offset() + accessor.offset();
    let end = base + if count == 0 { 0 } else { (count - 1) * stride + 3 * size };
    ensure!(
        end <= view.offset() + view.length() && end <= data.len(),
        "accessor {} overruns",
        accessor.index()
    );
    let norm = accessor.normalized();
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let mut v = [0f32; 3];
        for (c, x) in v.iter_mut().enumerate() {
            let at = base + i * stride + c * size;
            let b = &data[at..at + size];
            *x = match ty {
                DataType::F32 => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                DataType::I8 => scale(b[0] as i8 as f32, 127.0, norm, true),
                DataType::U8 => scale(b[0] as f32, 255.0, norm, false),
                DataType::I16 => {
                    scale(i16::from_le_bytes([b[0], b[1]]) as f32, 32767.0, norm, true)
                }
                DataType::U16 => {
                    scale(u16::from_le_bytes([b[0], b[1]]) as f32, 65535.0, norm, false)
                }
                DataType::U32 => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32,
            };
        }
        out.push(v);
    }
    Ok(out)
}

fn scale(v: f32, max: f32, normalized: bool, signed: bool) -> f32 {
    if !normalized {
        return v;
    }
    let s = v / max;
    if signed {
        s.max(-1.0)
    } else {
        s
    }
}
