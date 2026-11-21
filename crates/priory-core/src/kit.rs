//! The factory's own skills, instructions and memories, as each run gets them (R11.1).

use priory_factory::factory::{Factory, Harness};
use priory_runtime::runtime;
use sha2::{Digest, Sha256};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// A hex digest of `parts`, each prefixed with its length so no two lists of parts hash alike.
fn digest<'a>(parts: impl IntoIterator<Item = &'a [u8]>) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part);
    }
    hasher.finalize()[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Each skill's name, then each of its files' paths and bytes, in order.
fn skill_parts(factory: &Factory) -> Vec<&[u8]> {
    let mut parts = Vec::new();
    for (name, skill) in &factory.skills {
        parts.push(name.as_bytes());
        for (path, bytes) in &skill.value.files {
            parts.extend([path.as_os_str().as_bytes(), bytes.as_slice()]);
        }
    }
    parts
}

/// Each memory's name and bytes, in order.
fn memory_parts(factory: &Factory) -> Vec<&[u8]> {
    let mut parts = Vec::new();
    for (name, memory) in &factory.memories {
        parts.extend([name.as_bytes(), memory.value.bytes.as_slice()]);
    }
    parts
}

/// A hash of the factory's skills, instructions and memories that a run of `harness` gets, or none
/// when it gets none of them. A Codex run gets no skills.
pub(crate) fn hash(factory: &Factory, harness: &Harness) -> Option<String> {
    let instructions = factory.instructions();
    let skills = matches!(harness, Harness::ClaudeCode) && !factory.skills.is_empty();
    let memories = !factory.memories.is_empty();
    if instructions.is_empty() && !skills && !memories {
        return None;
    }
    let mut parts: Vec<&[u8]> = vec![b"instructions", instructions.as_bytes()];
    if skills {
        parts.push(b"skills");
        parts.extend(skill_parts(factory));
    }
    // Without memories, the hash is the one runs recorded before them.
    if memories {
        parts.push(b"memories");
        parts.extend(memory_parts(factory));
    }
    Some(digest(parts))
}

/// The folder that holds the factory's memories, `<home>/kit/<hash>/memories`, written once per
/// content, or none for a factory without memories.
pub(crate) fn memories(factory: &Factory, home: &Path) -> Result<Option<PathBuf>, String> {
    if factory.memories.is_empty() {
        return Ok(None);
    }
    // The tag keeps this folder's hash apart from a plugin folder's.
    let hash = digest([&b"memories"[..]].into_iter().chain(memory_parts(factory)));
    let files: Vec<(PathBuf, &[u8])> = factory
        .memories
        .iter()
        .map(|(name, memory)| {
            (
                PathBuf::from(format!("{name}.md")),
                memory.value.bytes.as_slice(),
            )
        })
        .collect();
    runtime::write_once(&home.join("kit").join(hash), "memories", &files).map(Some)
}

/// The plugin folder that holds the factory's skills, `<home>/kit/<hash>/plugin`, written once per
/// content, or none for a factory without skills.
pub(crate) fn plugin(factory: &Factory, home: &Path) -> Result<Option<PathBuf>, String> {
    if factory.skills.is_empty() {
        return Ok(None);
    }
    let hash = digest(skill_parts(factory));
    let manifest = serde_json::json!({
        "name": "priory",
        "description": format!("The skills of the Priory factory {}", factory.name),
    })
    .to_string();
    let mut files: Vec<(PathBuf, &[u8])> = vec![(
        PathBuf::from(".claude-plugin/plugin.json"),
        manifest.as_bytes(),
    )];
    for (name, skill) in &factory.skills {
        for (path, bytes) in &skill.value.files {
            files.push((Path::new("skills").join(name).join(path), bytes));
        }
    }
    runtime::plugin(&home.join("kit").join(hash), &files).map(Some)
}
