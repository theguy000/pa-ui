use super::*;

thread_local! {
    static MESSAGE_CACHE: std::cell::RefCell<std::collections::HashMap<String, CachedMessage>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

#[must_use]
/// What a message row-set depends on, so an unchanged message is not parsed
/// again: a poll every 900 ms was re-running markdown and highlighting for all
/// three hundred messages of a page and then throwing most of it away.
#[derive(Clone)]
pub(crate) struct CachedMessage {
    fingerprint: u64,
    rows: Vec<Row>,
}


/// A message plus every tool result it owns. The result is what a cell row
/// draws, so a result that arrives late must change the fingerprint.
pub(crate) fn fingerprint(
    message: &Value,
    outputs: &std::collections::HashMap<String, (String, bool)>,
    bashes: &std::collections::HashMap<String, BashInfo>,
    durations: &std::collections::HashMap<String, u64>,
) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    message.to_string().hash(&mut hasher);
    if let Some(blocks) = message.get("content").and_then(Value::as_array) {
        for block in blocks {
            if block.get("type").and_then(Value::as_str) != Some("toolCall") {
                continue;
            }
            let id = block.get("id").and_then(Value::as_str).unwrap_or_default();
            if let Some((text, failed)) = outputs.get(id) {
                text.hash(&mut hasher);
                failed.hash(&mut hasher);
            }
            if let Some(info) = bashes.get(id) {
                info.first.hash(&mut hasher);
                info.count.hash(&mut hasher);
            }
            durations.get(id).hash(&mut hasher);
        }
    }
    hasher.finish()
}


/// Drop the cache when it grows past what a very long session could hold, so a
/// window left open all day cannot creep upward without bound.
pub(crate) fn cache_put(owner: &str, fingerprint: u64, rows: &[Row]) {
    MESSAGE_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() > 4096 {
            cache.clear();
        }
        cache.insert(
            owner.to_string(),
            CachedMessage {
                fingerprint,
                rows: rows.to_vec(),
            },
        );
    });
}


/// Empty the per-message cache. Only a measurement needs this: the cache is
/// bounded on its own and follows the messages it is keyed by.
#[cfg(test)]
pub fn reset_cache() {
    MESSAGE_CACHE.with(|cache| cache.borrow_mut().clear());
}


pub(crate) fn cache_get(owner: &str, fingerprint: u64) -> Option<Vec<Row>> {
    MESSAGE_CACHE.with(|cache| {
        let cache = cache.borrow();
        match cache.get(owner) {
            Some(entry) if entry.fingerprint == fingerprint => Some(entry.rows.clone()),
            _ => None,
        }
    })
}
