use std::borrow::Cow;

use nanocode_durability_core::StoreError;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

#[derive(Serialize)]
#[cfg_attr(not(target_family = "wasm"), derive(Deserialize))]
#[serde(untagged)]
enum Edit<'a> {
    Copy(usize, usize),
    Insert(Cow<'a, str>),
}

#[cfg(target_family = "wasm")]
impl<'de> Deserialize<'de> for Edit<'_> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Edit<'static>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("untagged enum Edit")
            }

            fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<Edit<'static>, E> {
                Ok(Edit::Insert(Cow::Owned(text.to_owned())))
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                seq: A,
            ) -> Result<Edit<'static>, A::Error> {
                <(usize, usize)>::deserialize(serde::de::value::SeqAccessDeserializer::new(seq))
                    .map(|(from, len)| Edit::Copy(from, len))
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}

const PRIME: u64 = 0x0000_0100_0000_01b3;
const SPREAD: u64 = 0x9e37_79b9_7f4a_7c15;

fn invalid(detail: impl std::fmt::Display) -> StoreError {
    StoreError::Backend(format!("turn log checkpoint patch is invalid: {detail}"))
}

fn copy_width(len: usize) -> usize {
    let digits = len.checked_ilog10().map_or(1, |digits| digits as usize + 1);
    2 * digits + "[,],".len()
}

fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0, |hash, byte| {
        hash.wrapping_mul(PRIME).wrapping_add(u64::from(*byte))
    })
}

fn common(left: &[u8], right: &[u8]) -> usize {
    let words = left
        .as_chunks::<8>()
        .0
        .iter()
        .zip(right.as_chunks::<8>().0)
        .take_while(|(left, right)| left == right)
        .count()
        * 8;
    words
        + left[words..]
            .iter()
            .zip(&right[words..])
            .take_while(|(left, right)| left == right)
            .count()
}

pub(crate) fn diff(base: &str, new: &str) -> Result<Option<Box<RawValue>>, StoreError> {
    let window = copy_width(base.len());
    let (old, now) = (base.as_bytes(), new.as_bytes());
    if old.len() < window || now.len() < window {
        return Ok(None);
    }
    let blocks = old.len() / window;
    let bits = (2 * blocks).next_power_of_two().trailing_zeros();
    let slot = |hash: u64| (hash.wrapping_mul(SPREAD) >> (u64::BITS - bits)) as usize;
    let top = (1..window).fold(1u64, |power, _| power.wrapping_mul(PRIME));
    let roll = |hash: u64, out: u8, into: u8| {
        hash.wrapping_sub(u64::from(out).wrapping_mul(top))
            .wrapping_mul(PRIME)
            .wrapping_add(u64::from(into))
    };
    let mut table = vec![(0, usize::MAX); 1 << bits];
    for (block, bytes) in old.chunks_exact(window).enumerate() {
        let hash = hash(bytes);
        let entry = &mut table[slot(hash)];
        if entry.1 == usize::MAX {
            *entry = (hash, block * window);
        }
    }
    let mut edits: Vec<Edit<'_>> = Vec::new();
    let mut literal = 0;
    let mut at = 0;
    let mut rolling = hash(&now[..window]);
    while at + window <= now.len() {
        let (seen, found) = table[slot(rolling)];
        if found != usize::MAX
            && seen == rolling
            && old[found..found + window] == now[at..at + window]
        {
            let (mut start, mut from) = (at, found);
            while start > literal && from > 0 && now[start - 1] == old[from - 1] {
                start -= 1;
                from -= 1;
            }
            while !new.is_char_boundary(start) {
                start += 1;
                from += 1;
            }
            let mut end = at + window;
            end += common(&now[end..], &old[from + (end - start)..]);
            while !new.is_char_boundary(end) {
                end -= 1;
            }
            if end >= start + window {
                if literal < start {
                    edits.push(Edit::Insert(Cow::Borrowed(&new[literal..start])));
                }
                match edits.last_mut() {
                    Some(Edit::Copy(previous, len))
                        if literal == start && *previous + *len == from =>
                    {
                        *len += end - start;
                    }
                    _ => edits.push(Edit::Copy(from, end - start)),
                }
                literal = end;
                at = end;
                if at + window <= now.len() {
                    rolling = hash(&now[at..at + window]);
                }
                continue;
            }
        }
        if at + window < now.len() {
            rolling = roll(rolling, now[at], now[at + window]);
        }
        at += 1;
    }
    if literal < new.len() {
        edits.push(Edit::Insert(Cow::Borrowed(&new[literal..])));
    }
    let edits = serde_json::value::to_raw_value(&edits).map_err(|error| {
        StoreError::NotCommitted(format!("encode turn log checkpoint patch: {error}"))
    })?;
    Ok((edits.get().len() < new.len()).then_some(edits))
}

pub(crate) fn apply(base: &str, edits: &RawValue) -> Result<String, StoreError> {
    let edits: Vec<Edit<'_>> = serde_json::from_str(edits.get()).map_err(invalid)?;
    let mut payload = String::new();
    for edit in &edits {
        match edit {
            Edit::Copy(from, len) => payload.push_str(
                from.checked_add(*len)
                    .and_then(|end| base.get(*from..end))
                    .ok_or_else(|| {
                        invalid(format!(
                            "it copies {len} bytes at {from} from a base of {} bytes",
                            base.len()
                        ))
                    })?,
            ),
            Edit::Insert(text) => payload.push_str(text),
        }
    }
    Ok(payload)
}
