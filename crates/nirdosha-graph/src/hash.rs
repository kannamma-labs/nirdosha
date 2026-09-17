use crate::{Error, Result};
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};

pub const MAX_INTEGER: u64 = 9_007_199_254_740_991;
pub fn bytes(bytes: &[u8]) -> String {
    nirdosha_audit::crypto_backend::sha256(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub fn structured(domain: &str, value: &Value) -> Result<String> {
    safe_numbers(value)?;
    let encoded = serde_jcs::to_vec(
        &json!({"hash_schema":"nirdosha.hi.hash/v1","domain":domain,"value":value}),
    )
    .map_err(|e| Error::new("SCHEMA_INVALID", e.to_string()))?;
    Ok(bytes(&encoded))
}
pub fn safe_numbers(v: &Value) -> Result<()> {
    match v {
        Value::Number(n)
            if n.as_u64().is_some_and(|n| n > MAX_INTEGER)
                || n.as_i64().is_some_and(|n| n.unsigned_abs() > MAX_INTEGER)
                || n.as_f64()
                    .is_some_and(|n| n.fract() == 0.0 && n.abs() > MAX_INTEGER as f64) =>
        {
            Err(Error::new(
                "SCHEMA_INVALID",
                "Exact large numeric values require schema-tagged strings",
            ))
        }
        Value::Array(xs) => {
            for x in xs {
                safe_numbers(x)?;
            }
            Ok(())
        }
        Value::Object(xs) => {
            for x in xs.values() {
                safe_numbers(x)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Parse before Value erases duplicate keys. Never execute a provider fragment.
pub fn parse(input: &str) -> Result<Value> {
    struct Strict(Value);
    impl<'de> Deserialize<'de> for Strict {
        fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
            struct V;
            impl<'de> Visitor<'de> for V {
                type Value = Strict;
                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    f.write_str("JSON with unique object keys")
                }
                fn visit_bool<E: de::Error>(self, v: bool) -> std::result::Result<Strict, E> {
                    Ok(Strict(v.into()))
                }
                fn visit_i64<E: de::Error>(self, v: i64) -> std::result::Result<Strict, E> {
                    Ok(Strict(v.into()))
                }
                fn visit_u64<E: de::Error>(self, v: u64) -> std::result::Result<Strict, E> {
                    Ok(Strict(v.into()))
                }
                fn visit_f64<E: de::Error>(self, v: f64) -> std::result::Result<Strict, E> {
                    serde_json::Number::from_f64(v)
                        .map(|n| Strict(Value::Number(n)))
                        .ok_or_else(|| E::custom("nonfinite number"))
                }
                fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<Strict, E> {
                    Ok(Strict(v.into()))
                }
                fn visit_string<E: de::Error>(self, v: String) -> std::result::Result<Strict, E> {
                    Ok(Strict(v.into()))
                }
                fn visit_unit<E: de::Error>(self) -> std::result::Result<Strict, E> {
                    Ok(Strict(Value::Null))
                }
                fn visit_seq<A: SeqAccess<'de>>(
                    self,
                    mut a: A,
                ) -> std::result::Result<Strict, A::Error> {
                    let mut out = Vec::new();
                    while let Some(v) = a.next_element::<Strict>()? {
                        out.push(v.0);
                    }
                    Ok(Strict(Value::Array(out)))
                }
                fn visit_map<A: MapAccess<'de>>(
                    self,
                    mut a: A,
                ) -> std::result::Result<Strict, A::Error> {
                    let mut out = serde_json::Map::new();
                    while let Some(k) = a.next_key::<String>()? {
                        if out.contains_key(&k) {
                            return Err(de::Error::custom(format!("duplicate key: {k}")));
                        }
                        out.insert(k, a.next_value::<Strict>()?.0);
                    }
                    Ok(Strict(Value::Object(out)))
                }
            }
            d.deserialize_any(V)
        }
    }
    let v = serde_json::from_str::<Strict>(input)?.0;
    safe_numbers(&v)?;
    Ok(v)
}
