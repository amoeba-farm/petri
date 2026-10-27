//! JSON as JavaScript's `JSON.stringify(JSON.parse(text))` writes it: object
//! keys in the order they came (integer-like keys first, ascending), a
//! repeated key's last value in its first place. A submission's retry key
//! covers its candidate written this way, as the Node service recorded it.

use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Number, Value};

enum Ordered {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<Ordered>),
    Object(Vec<(String, Ordered)>),
}

impl<'de> Deserialize<'de> for Ordered {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Any;
        impl<'de> Visitor<'de> for Any {
            type Value = Ordered;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("JSON")
            }
            fn visit_unit<E>(self) -> Result<Ordered, E> {
                Ok(Ordered::Null)
            }
            fn visit_bool<E>(self, value: bool) -> Result<Ordered, E> {
                Ok(Ordered::Bool(value))
            }
            fn visit_u64<E>(self, value: u64) -> Result<Ordered, E> {
                Ok(Ordered::Number(value.into()))
            }
            fn visit_i64<E>(self, value: i64) -> Result<Ordered, E> {
                Ok(Ordered::Number(value.into()))
            }
            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Ordered, E> {
                Number::from_f64(value)
                    .map(Ordered::Number)
                    .ok_or_else(|| E::custom("a finite number"))
            }
            fn visit_str<E>(self, value: &str) -> Result<Ordered, E> {
                Ok(Ordered::String(value.into()))
            }
            fn visit_string<E>(self, value: String) -> Result<Ordered, E> {
                Ok(Ordered::String(value))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Ordered, A::Error> {
                let mut items = Vec::new();
                while let Some(item) = seq.next_element()? {
                    items.push(item);
                }
                Ok(Ordered::Array(items))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Ordered, A::Error> {
                let mut entries: Vec<(String, Ordered)> = Vec::new();
                while let Some((key, value)) = map.next_entry::<String, Ordered>()? {
                    match entries.iter_mut().find(|(k, _)| *k == key) {
                        Some(entry) => entry.1 = value,
                        None => entries.push((key, value)),
                    }
                }
                Ok(Ordered::Object(entries))
            }
        }
        deserializer.deserialize_any(Any)
    }
}

/// An array index as JavaScript orders object keys: a canonical integer
/// below 2^32 - 1.
fn index(key: &str) -> Option<u32> {
    let n: u32 = key.parse().ok()?;
    (n < u32::MAX && n.to_string() == key).then_some(n)
}

fn write(value: &Ordered, out: &mut String) {
    match value {
        Ordered::Null => out.push_str("null"),
        Ordered::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Ordered::Number(n) => out.push_str(&n.to_string()),
        Ordered::String(s) => out.push_str(&Value::String(s.clone()).to_string()),
        Ordered::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write(item, out);
            }
            out.push(']');
        }
        Ordered::Object(entries) => {
            let mut keys: Vec<&(String, Ordered)> =
                entries.iter().filter(|(k, _)| index(k).is_some()).collect();
            keys.sort_by_key(|(k, _)| index(k));
            keys.extend(entries.iter().filter(|(k, _)| index(k).is_none()));
            out.push('{');
            for (i, (key, item)) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(key.clone()).to_string());
                out.push(':');
                write(item, out);
            }
            out.push('}');
        }
    }
}

/// The member `key` of the JSON object `text`, written as JavaScript would.
pub fn member(text: &str, key: &str) -> Option<String> {
    let Ok(Ordered::Object(entries)) = serde_json::from_str::<Ordered>(text) else {
        return None;
    };
    let (_, value) = entries.iter().find(|(k, _)| k == key)?;
    let mut out = String::new();
    write(value, &mut out);
    Some(out)
}
