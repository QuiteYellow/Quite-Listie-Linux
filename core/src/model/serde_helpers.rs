use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

// ---------------------------------------------------------------------------
// Forward-compatible field decoding (Swift `decodeOptionalPreserving`)
// ---------------------------------------------------------------------------
//
// `ListItem` / `ListSummary` deserialize through a raw `serde_json::Map` so that an
// unparseable *value* of a known field (e.g. a future `ReminderRepeatUnit`, a malformed
// date) is preserved across a round-trip instead of failing the whole document parse. The
// raw value is left in the map and ends up in the struct's flattened `extra`, exactly as
// Swift stashes it in `_preserved`.

/// Decode a required field, erroring if it is absent or unparseable.
pub(crate) fn required<T, E>(map: &mut Map<String, Value>, key: &str) -> Result<T, E>
where
    T: DeserializeOwned,
    E: serde::de::Error,
{
    let v = map
        .remove(key)
        .ok_or_else(|| E::custom(format!("missing field `{key}`")))?;
    serde_json::from_value(v).map_err(E::custom)
}

/// Decode an optional field, *preserving* an unparseable value: on parse failure the raw
/// JSON stays in `map` (so it round-trips via the struct's flattened `extra`) and `None` is
/// returned. For fields skipped from output when empty/`None`, so there is no duplicate key.
pub(crate) fn optional_preserving<T>(map: &mut Map<String, Value>, key: &str) -> Option<T>
where
    T: DeserializeOwned,
{
    match map.get(key) {
        None => None,
        Some(Value::Null) => {
            map.remove(key);
            None
        }
        Some(v) => match serde_json::from_value::<T>(v.clone()) {
            Ok(parsed) => {
                map.remove(key);
                Some(parsed)
            }
            Err(_) => None,
        },
    }
}

/// Decode an always-serialized field with a default, *dropping* an unparseable value.
/// Keeping it would emit a duplicate key (the field itself always serializes), and a
/// malformed scalar of such a field carries no meaning worth preserving.
pub(crate) fn optional_lenient<T>(map: &mut Map<String, Value>, key: &str) -> Option<T>
where
    T: DeserializeOwned,
{
    match map.remove(key) {
        None | Some(Value::Null) => None,
        Some(v) => serde_json::from_value(v).ok(),
    }
}

/// ISO-8601 UTC datetime — matches the Swift JSONEncoder default.
pub mod iso8601 {
    use super::*;

    pub fn serialize<S>(dt: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
            .serialize(s)
    }

    pub fn deserialize<'de, D>(d: D) -> Result<DateTime<Utc>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(d)?;
        raw.parse::<DateTime<Utc>>()
            .map_err(serde::de::Error::custom)
    }
}

/// Optional ISO-8601 UTC datetime.
pub mod iso8601_opt {
    use super::*;

    pub fn serialize<S>(dt: &Option<DateTime<Utc>>, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match dt {
            Some(dt) => dt
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
                .serialize(s),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(d: D) -> Result<Option<DateTime<Utc>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw: Option<String> = Option::deserialize(d)?;
        match raw {
            None => Ok(None),
            Some(s) => s
                .parse::<DateTime<Utc>>()
                .map(Some)
                .map_err(serde::de::Error::custom),
        }
    }
}

/// Quantity: serialize as integer when the value is whole, float otherwise.
/// Matches the Swift JSONEncoder quantity encoding.
pub mod quantity {
    use super::*;

    pub fn serialize<S>(qty: &f64, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if qty.fract() == 0.0 && qty.is_finite() {
            (*qty as i64).serialize(s)
        } else {
            qty.serialize(s)
        }
    }

    pub fn deserialize<'de, D>(d: D) -> Result<f64, D::Error>
    where
        D: Deserializer<'de>,
    {
        Value::deserialize(d)?
            .as_f64()
            .ok_or_else(|| serde::de::Error::custom("expected number for quantity"))
    }
}
