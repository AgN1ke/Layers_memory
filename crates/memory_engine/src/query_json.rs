//! Decode JSON queries while reporting fields the typed request ignored.

use std::collections::BTreeSet;

use serde::de::DeserializeOwned;
use serde::Deserializer;
use serde_ignored::Path;
use serde_json::Value;

/// Decode one JSON request and report ignored fields as response notes.
pub fn from_str<T: DeserializeOwned>(raw: &str) -> serde_json::Result<(T, Vec<String>)> {
    let mut deserializer = serde_json::Deserializer::from_str(raw);
    let parsed = deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(parsed)
}

/// Decode a request after host-side JSON normalization.
pub fn from_value<T: DeserializeOwned>(value: Value) -> serde_json::Result<(T, Vec<String>)> {
    deserialize(value)
}

fn deserialize<'de, D, T>(deserializer: D) -> Result<(T, Vec<String>), D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let mut paths = BTreeSet::new();
    let request = serde_ignored::deserialize(deserializer, |path| {
        let mut pointer = String::new();
        append_pointer(&mut pointer, &path);
        paths.insert(pointer);
    })?;
    let notes = paths
        .into_iter()
        .map(|path| format!("ignored unknown request field: {}", Value::String(path)))
        .collect();
    Ok((request, notes))
}

fn append_pointer(pointer: &mut String, path: &Path<'_>) {
    match path {
        Path::Root => {}
        Path::Seq { parent, index } => {
            append_pointer(pointer, parent);
            pointer.push('/');
            pointer.push_str(&index.to_string());
        }
        Path::Map { parent, key } => {
            append_pointer(pointer, parent);
            pointer.push('/');
            for character in key.chars() {
                match character {
                    '~' => pointer.push_str("~0"),
                    '/' => pointer.push_str("~1"),
                    _ => pointer.push(character),
                }
            }
        }
        Path::Some { parent }
        | Path::NewtypeStruct { parent }
        | Path::NewtypeVariant { parent } => {
            append_pointer(pointer, parent);
        }
    }
}
