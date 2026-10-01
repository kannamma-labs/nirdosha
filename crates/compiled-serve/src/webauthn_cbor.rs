//! A minimal CBOR (RFC 8949) decoder, scoped to exactly the subset
//! WebAuthn's `attestationObject`/`authenticatorData`/COSE `EC2` key
//! actually use: unsigned/negative integers, byte strings, text
//! strings, arrays, and maps, with the standard short/1/2/4/8-byte
//! length-follows encoding. Deliberately not a general-purpose CBOR
//! crate: major type 6 (tags) and 7 (floats/simple/bool/null/break) never
//! appear in a `none`-attestation WebAuthn object, so this decoder
//! rejects them outright rather than silently accepting and ignoring
//! bytes it doesn't actually understand — the same "fail closed on the
//! unexpected shape" posture `crates/nirdosha-guard-core`'s own
//! `RdbmsEmitter` was fixed to use this session (a wildcard match that
//! quietly did nothing was the actual bug there).

#[derive(Debug, Clone, PartialEq)]
pub enum CborValue {
    Uint(u64),
    /// The actual negative value, already applied (CBOR encodes `-1-n`
    /// for major type 1's argument `n`; this is `-1-n`, not `n`).
    Nint(i64),
    Bytes(Vec<u8>),
    Text(String),
    Array(Vec<CborValue>),
    Map(Vec<(CborValue, CborValue)>),
}

impl CborValue {
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            CborValue::Bytes(b) => Some(b),
            _ => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            CborValue::Text(t) => Some(t),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            CborValue::Uint(u) => i64::try_from(*u).ok(),
            CborValue::Nint(n) => Some(*n),
            _ => None,
        }
    }

    /// Looks up a map's value by a text key. `None` if this isn't a
    /// map, or the key isn't present.
    pub fn get_text_key(&self, key: &str) -> Option<&CborValue> {
        match self {
            CborValue::Map(entries) => entries.iter().find(|(k, _)| k.as_text() == Some(key)).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Looks up a map's value by an integer key — COSE keys are
    /// integers (`1` = kty, `-1` = crv, `-2` = x, `-3` = y, ...), never
    /// text.
    pub fn get_int_key(&self, key: i64) -> Option<&CborValue> {
        match self {
            CborValue::Map(entries) => entries.iter().find(|(k, _)| k.as_int() == Some(key)).map(|(_, v)| v),
            _ => None,
        }
    }
}

/// Decodes exactly one CBOR item from the start of `input`, returning it
/// plus the number of bytes consumed. Callers that expect the item to
/// span the whole buffer (attestation objects and COSE keys both do)
/// should check the returned length themselves — this function
/// deliberately doesn't assume that for itself, since a nested call
/// (e.g. decoding a map's own values) needs the partial-consumption
/// behavior.
pub fn decode_one(input: &[u8]) -> Result<(CborValue, usize), String> {
    let Some(&initial) = input.first() else { return Err("CBOR: unexpected end of input".to_string()) };
    let major = initial >> 5;
    let arg_info = initial & 0x1F;

    let (arg, header_len) = decode_argument(input, arg_info)?;

    match major {
        0 => Ok((CborValue::Uint(arg), header_len)),
        1 => {
            let value = i64::try_from(arg).map_err(|_| "CBOR: negative integer magnitude out of i64 range".to_string())?;
            Ok((CborValue::Nint(-1 - value), header_len))
        }
        2 => {
            let len = arg as usize;
            let end = header_len.checked_add(len).ok_or_else(|| "CBOR: byte string length overflow".to_string())?;
            let bytes = input.get(header_len..end).ok_or_else(|| "CBOR: byte string runs past end of input".to_string())?;
            Ok((CborValue::Bytes(bytes.to_vec()), end))
        }
        3 => {
            let len = arg as usize;
            let end = header_len.checked_add(len).ok_or_else(|| "CBOR: text string length overflow".to_string())?;
            let bytes = input.get(header_len..end).ok_or_else(|| "CBOR: text string runs past end of input".to_string())?;
            let text = std::str::from_utf8(bytes).map_err(|_| "CBOR: text string is not valid UTF-8".to_string())?;
            Ok((CborValue::Text(text.to_string()), end))
        }
        4 => {
            let count = arg as usize;
            let mut offset = header_len;
            let mut items = Vec::with_capacity(count);
            for _ in 0..count {
                let (item, consumed) = decode_one(input.get(offset..).ok_or_else(|| "CBOR: array runs past end of input".to_string())?)?;
                items.push(item);
                offset += consumed;
            }
            Ok((CborValue::Array(items), offset))
        }
        5 => {
            let count = arg as usize;
            let mut offset = header_len;
            let mut entries = Vec::with_capacity(count);
            for _ in 0..count {
                let (key, key_len) = decode_one(input.get(offset..).ok_or_else(|| "CBOR: map key runs past end of input".to_string())?)?;
                offset += key_len;
                let (value, value_len) = decode_one(input.get(offset..).ok_or_else(|| "CBOR: map value runs past end of input".to_string())?)?;
                offset += value_len;
                entries.push((key, value));
            }
            Ok((CborValue::Map(entries), offset))
        }
        _ => Err(format!("CBOR: major type {major} is out of scope for this decoder (tags, floats, and simple values never appear in a `none`-attestation WebAuthn object) — refusing rather than silently skipping")),
    }
}

/// Decodes major type 0's own argument encoding (shared by every major
/// type's length/value field): the low 5 bits of the initial byte are
/// either the value itself (0-23), or a marker for how many following
/// bytes encode it (24/25/26/27 => 1/2/4/8 bytes, big-endian). Returns
/// the decoded argument and the total header length (1 + however many
/// follow-on bytes were consumed).
fn decode_argument(input: &[u8], arg_info: u8) -> Result<(u64, usize), String> {
    match arg_info {
        0..=23 => Ok((arg_info as u64, 1)),
        24 => {
            let byte = *input.get(1).ok_or_else(|| "CBOR: truncated 1-byte length".to_string())?;
            Ok((byte as u64, 2))
        }
        25 => {
            let bytes: [u8; 2] = input.get(1..3).ok_or_else(|| "CBOR: truncated 2-byte length".to_string())?.try_into().unwrap();
            Ok((u16::from_be_bytes(bytes) as u64, 3))
        }
        26 => {
            let bytes: [u8; 4] = input.get(1..5).ok_or_else(|| "CBOR: truncated 4-byte length".to_string())?.try_into().unwrap();
            Ok((u32::from_be_bytes(bytes) as u64, 5))
        }
        27 => {
            let bytes: [u8; 8] = input.get(1..9).ok_or_else(|| "CBOR: truncated 8-byte length".to_string())?.try_into().unwrap();
            Ok((u64::from_be_bytes(bytes), 9))
        }
        _ => Err(format!("CBOR: additional-info {arg_info} (indefinite-length/reserved) is out of scope for this decoder")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_small_unsigned_int() {
        let (value, len) = decode_one(&[0x05]).unwrap();
        assert_eq!(value, CborValue::Uint(5));
        assert_eq!(len, 1);
    }

    #[test]
    fn decodes_a_1_byte_length_unsigned_int() {
        // 0x18 = major 0, arg_info 24 (1-byte length follows); 0xFF = 255.
        let (value, _) = decode_one(&[0x18, 0xFF]).unwrap();
        assert_eq!(value, CborValue::Uint(255));
    }

    #[test]
    fn decodes_negative_integers_the_cose_key_uses() {
        // COSE's crv key is -1: major 1, arg_info 0 -> -1-0 = -1.
        let (value, _) = decode_one(&[0x20]).unwrap();
        assert_eq!(value, CborValue::Nint(-1));
        // COSE's x key is -2: major 1, arg_info 1 -> -1-1 = -2.
        let (value, _) = decode_one(&[0x21]).unwrap();
        assert_eq!(value, CborValue::Nint(-2));
        // alg = -7 (ES256): major 1, arg_info 6 -> -1-6 = -7.
        let (value, _) = decode_one(&[0x26]).unwrap();
        assert_eq!(value, CborValue::Nint(-7));
    }

    #[test]
    fn decodes_a_byte_string() {
        let (value, len) = decode_one(&[0x43, 0x01, 0x02, 0x03]).unwrap();
        assert_eq!(value, CborValue::Bytes(vec![1, 2, 3]));
        assert_eq!(len, 4);
    }

    #[test]
    fn decodes_a_text_string() {
        let (value, _) = decode_one(&[0x64, b'n', b'o', b'n', b'e']).unwrap();
        assert_eq!(value, CborValue::Text("none".to_string()));
    }

    #[test]
    fn decodes_a_map_with_mixed_key_types_like_a_cose_ec2_key() {
        // {1: 2, 3: -7, -1: 1} -- kty=EC2, alg=ES256, crv=P-256.
        let bytes = [
            0xA3, // map(3)
            0x01, 0x02, // 1: 2
            0x03, 0x26, // 3: -7
            0x20, 0x01, // -1: 1
        ];
        let (value, len) = decode_one(&bytes).unwrap();
        assert_eq!(len, bytes.len());
        assert_eq!(value.get_int_key(1), Some(&CborValue::Uint(2)));
        assert_eq!(value.get_int_key(3), Some(&CborValue::Nint(-7)));
        assert_eq!(value.get_int_key(-1), Some(&CborValue::Uint(1)));
    }

    #[test]
    fn decodes_a_map_with_text_keys_like_an_attestation_object() {
        // {"fmt": "none"}
        let bytes = [0xA1, 0x63, b'f', b'm', b't', 0x64, b'n', b'o', b'n', b'e'];
        let (value, _) = decode_one(&bytes).unwrap();
        assert_eq!(value.get_text_key("fmt").and_then(CborValue::as_text), Some("none"));
    }

    #[test]
    fn refuses_a_float_or_simple_value_rather_than_silently_skipping_it() {
        // 0xF5 = major 7 (simple), value `true` -- out of scope, must error.
        let err = decode_one(&[0xF5]).unwrap_err();
        assert!(err.contains("major type 7"), "error should name the actual unsupported major type: {err}");
    }

    #[test]
    fn truncated_input_is_an_error_not_a_panic() {
        assert!(decode_one(&[0x43, 0x01]).is_err(), "a byte string claiming 3 bytes but supplying 1 must error, not panic or read past the buffer");
        assert!(decode_one(&[]).is_err());
    }
}
