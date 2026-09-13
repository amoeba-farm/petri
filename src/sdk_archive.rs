//! Bounded lossless archive representation; no filesystem or process authority.
//! V1 file streams and V2 shared blocks normalize to the same extraction plan.

use super::unavailable;
use crate::{backend::CliError, content_hash::sha256_hex as hash};
use serde_json::Value;
use std::collections::BTreeMap;

const MAX_FILE_BYTES: usize = 128 * 1024 * 1024;
const MAX_HEADER_BYTES: usize = 16 * 1024 * 1024;
const COMPRESSED_HEADER: u64 = 1 << 63;

pub(super) struct Member {
    pub index: usize,
    pub offset: usize,
    size: usize,
    digest: String,
}

pub(super) struct Block {
    pub offset: usize,
    length: usize,
    size: usize,
    digest: String,
    pub files: Vec<Member>,
}

pub(super) struct Archive<'a> {
    pub manifest: Value,
    pub blocks: Vec<Block>,
    payload: &'a [u8],
}

fn number(value: &Value, maximum: usize) -> Result<usize, CliError> {
    value
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .filter(|n| *n <= maximum)
        .ok_or_else(|| unavailable("SDK archive size invalid"))
}

fn digest(value: &Value) -> Result<String, CliError> {
    value
        .as_str()
        .filter(|s| {
            s.len() == 64
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
        .map(str::to_owned)
        .ok_or_else(|| unavailable("SDK file digest missing"))
}

fn inflate(bytes: &[u8], maximum: usize, exact: Option<usize>) -> Result<Vec<u8>, CliError> {
    // One extra byte detects expansion beyond the declared size. StreamEnd and
    // total_in also reject truncated streams and trailing compressed data.
    let mut decoded = Vec::with_capacity(maximum + 1);
    let mut decoder = flate2::Decompress::new(true);
    let status = decoder
        .decompress_vec(bytes, &mut decoded, flate2::FlushDecompress::Finish)
        .map_err(|_| unavailable("SDK archive decompression failed"))?;
    if status != flate2::Status::StreamEnd
        || decoder.total_in() != bytes.len() as u64
        || decoded.len() > maximum
        || exact.is_some_and(|size| size != decoded.len())
    {
        return Err(unavailable("SDK archive decompression failed"));
    }
    Ok(decoded)
}

impl<'a> Archive<'a> {
    pub(super) fn parse(bundle: &'a [u8]) -> Result<Self, CliError> {
        let word = bundle
            .get(..8)
            .ok_or_else(|| unavailable("SDK archive header invalid"))?;
        let word = u64::from_le_bytes(word.try_into().unwrap());
        let compressed = word & COMPRESSED_HEADER != 0;
        let length = usize::try_from(word & !COMPRESSED_HEADER)
            .ok()
            .filter(|n| *n <= MAX_HEADER_BYTES)
            .ok_or_else(|| unavailable("SDK archive header invalid"))?;
        let end = 8 + length;
        let header = bundle
            .get(8..end)
            .ok_or_else(|| unavailable("SDK archive header invalid"))?;
        let decoded;
        let header = if compressed {
            decoded = inflate(header, MAX_HEADER_BYTES, None)?;
            decoded.as_slice()
        } else {
            header
        };
        let manifest: Value = serde_json::from_slice(header)
            .map_err(|_| unavailable("SDK archive manifest invalid"))?;
        let version = manifest["schemaVersion"].as_u64().unwrap_or(0);
        if !matches!(version, 1 | 2) || (compressed && version != 2) {
            return Err(unavailable("SDK archive identity differs from Petri"));
        }
        let entries = manifest["files"]
            .as_array()
            .filter(|files| !files.is_empty() && files.len() <= 40000)
            .ok_or_else(|| unavailable("SDK archive file inventory invalid"))?;
        let payload = &bundle[end..];
        let mut blocks: BTreeMap<(usize, usize), Block> = BTreeMap::new();
        let mut expanded = 0usize;
        for (index, entry) in entries.iter().enumerate() {
            let size = number(&entry["bytes"], MAX_FILE_BYTES)?;
            expanded = expanded
                .checked_add(size)
                .filter(|n| *n <= 2 * 1024 * 1024 * 1024)
                .ok_or_else(|| unavailable("SDK archive expanded size invalid"))?;
            let offset = number(&entry["offset"], payload.len())?;
            let length = number(&entry["length"], MAX_FILE_BYTES + 64 * 1024)?;
            if length == 0
                || offset
                    .checked_add(length)
                    .is_none_or(|end| end > payload.len())
            {
                return Err(unavailable("SDK archive truncated"));
            }
            let file_digest = digest(&entry["sha256"])?;
            let (block_size, block_digest, slice) = if version == 1 {
                (size, file_digest.clone(), 0)
            } else {
                (
                    number(&entry["blockBytes"], MAX_FILE_BYTES)?,
                    digest(&entry["blockSha256"])?,
                    number(&entry["blockOffset"], MAX_FILE_BYTES)?,
                )
            };
            if slice.checked_add(size).is_none_or(|end| end > block_size) {
                return Err(unavailable("SDK archive offset invalid"));
            }
            let block = blocks.entry((offset, length)).or_insert_with(|| Block {
                offset,
                length,
                size: block_size,
                digest: block_digest.clone(),
                files: Vec::new(),
            });
            if block.size != block_size || block.digest != block_digest {
                return Err(unavailable("SDK archive content digest mismatch"));
            }
            block.files.push(Member {
                index,
                offset: slice,
                size,
                digest: file_digest,
            });
        }
        let blocks: Vec<_> = blocks.into_values().collect();
        let mut stored_end = 0;
        for block in &blocks {
            if block.offset != stored_end {
                return Err(unavailable("SDK archive offset invalid"));
            }
            stored_end += block.length;
            let mut members: Vec<_> = block.files.iter().collect();
            members.sort_by_key(|file| (file.offset, file.size));
            let mut decoded_end = 0;
            let mut previous: Option<&Member> = None;
            for member in members {
                if previous.is_some_and(|prior| {
                    prior.offset == member.offset
                        && prior.size == member.size
                        && prior.digest == member.digest
                }) {
                    continue; // Exact aliases are legal, partial overlaps are not.
                }
                if member.offset != decoded_end {
                    return Err(unavailable("SDK archive offset invalid"));
                }
                decoded_end += member.size;
                previous = Some(member);
            }
            if decoded_end != block.size {
                return Err(unavailable("SDK archive size invalid"));
            }
        }
        if stored_end != payload.len() {
            return Err(unavailable("SDK archive size invalid"));
        }
        Ok(Self {
            manifest,
            blocks,
            payload,
        })
    }

    pub(super) fn decode(&self, block: &Block) -> Result<Vec<u8>, CliError> {
        let decoded = inflate(
            &self.payload[block.offset..block.offset + block.length],
            block.size,
            Some(block.size),
        )?;
        if hash(&decoded) != block.digest {
            return Err(unavailable("SDK archive content digest mismatch"));
        }
        Ok(decoded)
    }
}
