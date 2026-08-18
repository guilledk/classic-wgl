//! Serde helpers that round-trip numeric arrays as base64-encoded JSON
//! strings, so tile / nav / height data inlines into `state.json` instead of
//! shipping as loose sidecar files.
//!
//! Encoding is `base64(JSON array)`, matching the old `map001.txt` /
//! `map001.nav.txt` sidecar convention.

use base64::Engine as _;

fn encode_json<T: serde::Serialize + ?Sized>(value: &T) -> String {
    let json = serde_json::to_string(value).expect("numeric array serializes to JSON");
    base64::engine::general_purpose::STANDARD.encode(json.as_bytes())
}

fn decode_json<T: serde::de::DeserializeOwned>(s: &str) -> Result<T, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(s.trim().as_bytes())
        .map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

/// Encode a `u32` slice as a base64 JSON-array string (used by the dumpers).
pub fn encode_u32(v: &[u32]) -> String {
    encode_json(v)
}

/// Encode an `f32` slice as a base64 JSON-array string (used by the dumpers).
pub fn encode_f32(v: &[f32]) -> String {
    encode_json(v)
}

/// `#[serde(with)]` module for `Vec<u32>`.
pub mod vec_u32 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &Vec<u32>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&super::encode_json(v))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u32>, D::Error> {
        // Accept `null` (treated as empty) in addition to a base64 string.
        match Option::<String>::deserialize(d)? {
            None => Ok(Vec::new()),
            Some(s) => super::decode_json(&s).map_err(serde::de::Error::custom),
        }
    }
}

/// `#[serde(with)]` module for `Vec<f32>`.
pub mod vec_f32 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &Vec<f32>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&super::encode_json(v))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<f32>, D::Error> {
        match Option::<String>::deserialize(d)? {
            None => Ok(Vec::new()),
            Some(s) => super::decode_json(&s).map_err(serde::de::Error::custom),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u32_round_trip() {
        let v = vec![0u32, 19, 9, 30];
        let encoded = encode_u32(&v);
        assert!(!encoded.is_empty());
        let decoded: Vec<u32> = decode_json(&encoded).unwrap();
        assert_eq!(decoded, v);
    }

    #[test]
    fn f32_round_trip() {
        let v = vec![0.0f32, 1.5, -2.25];
        let encoded = encode_f32(&v);
        let decoded: Vec<f32> = decode_json(&encoded).unwrap();
        assert_eq!(decoded, v);
    }

    #[derive(serde::Deserialize)]
    struct Holder {
        #[serde(default, with = "crate::serde_base64::vec_u32")]
        data: Vec<u32>,
    }

    #[test]
    fn null_deserializes_to_empty() {
        let h: Holder = serde_json::from_str(r#"{"data": null}"#).unwrap();
        assert!(h.data.is_empty());
    }

    #[test]
    fn missing_deserializes_to_empty() {
        let h: Holder = serde_json::from_str(r#"{}"#).unwrap();
        assert!(h.data.is_empty());
    }

    #[test]
    fn base64_deserializes_to_array() {
        let encoded = encode_u32(&[1, 2, 3]);
        let json = format!(r#"{{"data": "{encoded}"}}"#);
        let h: Holder = serde_json::from_str(&json).unwrap();
        assert_eq!(h.data, vec![1, 2, 3]);
    }
}
