//! Bulk rename of image files using name templates.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::naming::{self, NameContext};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Case {
    #[default]
    Keep,
    Lower,
    Upper,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortBy {
    #[default]
    Name,
    Date,
    Size,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RenameOptions {
    /// Template without extension, e.g. `holiday-{n:3}`.
    pub pattern: String,
    pub start: u64,
    pub step: u64,
    pub case: Case,
    /// Normalize extensions (`JPEG` -> `jpg`).
    pub normalize_ext: bool,
    pub sort: SortBy,
    /// Text to find in the produced name and replace (applied after the template).
    pub find: Option<String>,
    pub replace: String,
}

impl Default for RenameOptions {
    fn default() -> Self {
        Self {
            pattern: "{name}".into(),
            start: 1,
            step: 1,
            case: Case::Keep,
            normalize_ext: false,
            sort: SortBy::Name,
            find: None,
            replace: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameItem {
    pub from: PathBuf,
    pub to: PathBuf,
    pub changed: bool,
    /// Set when the target exists and is not part of this rename.
    pub conflict: bool,
}

/// Image files in `dir`, sorted by name.
pub fn collect_images(dir: &Path, recursive: bool) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    collect_into(dir, recursive, &mut out)?;
    out.sort_by_key(|p| p.to_string_lossy().to_lowercase());
    Ok(out)
}

fn collect_into(dir: &Path, recursive: bool, out: &mut Vec<PathBuf>) -> Result<()> {
    let entries = std::fs::read_dir(dir).with_context(|| format!("cannot read {}", dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            if recursive {
                collect_into(&path, true, out)?;
            }
        } else if photokit_codecs::is_supported_input(&path) {
            out.push(path);
        }
    }
    Ok(())
}

pub fn plan(files: &[PathBuf], opts: &RenameOptions) -> Result<Vec<RenameItem>> {
    if opts.pattern.trim().is_empty() {
        bail!("rename pattern is empty");
    }
    let mut files = files.to_vec();
    match opts.sort {
        SortBy::Name => files.sort_by_key(|p| p.to_string_lossy().to_lowercase()),
        SortBy::Date => files.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok()),
        SortBy::Size => files.sort_by_key(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)),
    }
    let dims = naming::needs_dimensions(&opts.pattern);
    let sources: HashSet<PathBuf> = files.iter().cloned().collect();
    let mut claimed: HashSet<PathBuf> = HashSet::new();
    let mut items = Vec::with_capacity(files.len());

    for (i, from) in files.iter().enumerate() {
        let mut ctx = NameContext::for_path(from, opts.start + i as u64 * opts.step.max(1));
        if dims {
            if let Some((w, h)) = photokit_codecs::probe_dimensions(from) {
                ctx.width = Some(w);
                ctx.height = Some(h);
            }
        }
        let mut stem = naming::render(&opts.pattern, &ctx);
        if let Some(find) = opts.find.as_deref().filter(|f| !f.is_empty()) {
            stem = naming::sanitize(&stem.replace(find, &opts.replace));
        }
        let mut ext = ctx.ext.clone();
        if opts.normalize_ext {
            ext = ext.to_ascii_lowercase();
            if ext == "jpeg" {
                ext = "jpg".into();
            }
        }
        let mut name = if ext.is_empty() { stem } else { format!("{stem}.{ext}") };
        name = match opts.case {
            Case::Keep => name,
            Case::Lower => name.to_lowercase(),
            Case::Upper => name.to_uppercase(),
        };
        let dir = from.parent().unwrap_or(Path::new("."));
        let wanted = dir.join(&name);
        // Collisions inside the batch get a counter; files that will be
        // renamed away are free to take.
        let to = naming::unique_path_with(&wanted, |p| {
            claimed.contains(&key(p)) || (p != from && p.exists() && !sources.contains(p) && !same_file(p, from))
        });
        claimed.insert(key(&to));
        let conflict = to != wanted;
        items.push(RenameItem { changed: &to != from, from: from.clone(), to, conflict });
    }
    Ok(items)
}

/// Case-insensitive key so `A.jpg` and `a.jpg` collide on macOS and Windows.
fn key(p: &Path) -> PathBuf {
    PathBuf::from(p.to_string_lossy().to_lowercase())
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// Apply a plan in two phases (to temporary names, then to targets) so
/// swaps like `a->b, b->a` work. Returns the number of renamed files.
pub fn apply(items: &[RenameItem]) -> Result<usize> {
    let todo: Vec<&RenameItem> = items.iter().filter(|i| i.changed).collect();
    let mut staged = Vec::with_capacity(todo.len());
    let pid = std::process::id();
    for (n, item) in todo.iter().enumerate() {
        let tmp = item.from.with_file_name(format!(".pk-rename-{pid}-{n}.tmp"));
        if let Err(e) = std::fs::rename(&item.from, &tmp) {
            rollback(&staged);
            return Err(e).with_context(|| format!("cannot rename {}", item.from.display()));
        }
        staged.push((tmp, *item));
    }
    for (i, (tmp, item)) in staged.iter().enumerate() {
        if item.to.exists() {
            rollback(&staged[i..]);
            bail!("{} already exists; stopped after {i} files", item.to.display());
        }
        std::fs::rename(tmp, &item.to).with_context(|| format!("cannot rename to {}", item.to.display()))?;
    }
    Ok(staged.len())
}

fn rollback(staged: &[(PathBuf, &RenameItem)]) {
    for (tmp, item) in staged {
        let _ = std::fs::rename(tmp, &item.from);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pk-rename-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn sequential_rename_and_swap() {
        let d = tempdir("seq");
        for n in ["b.jpg", "a.jpg", "c.PNG"] {
            std::fs::write(d.join(n), n).unwrap();
        }
        let files = collect_images(&d, false).unwrap();
        let opts = RenameOptions { pattern: "x-{n:2}".into(), normalize_ext: true, ..Default::default() };
        let plan = super::plan(&files, &opts).unwrap();
        let names: Vec<_> = plan.iter().map(|i| i.to.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["x-01.jpg", "x-02.jpg", "x-03.png"]);
        assert_eq!(apply(&plan).unwrap(), 3);
        assert_eq!(std::fs::read_to_string(d.join("x-01.jpg")).unwrap(), "a.jpg");

        // Reverse order swap: names are taken by other files in the same set.
        let files = collect_images(&d, false).unwrap();
        let opts = RenameOptions { pattern: "x-{n:2}".into(), start: 3, step: 0, ..Default::default() };
        let plan = super::plan(&files, &opts).unwrap();
        assert!(plan.iter().all(|i| !i.to.exists() || files.contains(&i.to)));
        apply(&plan).unwrap();
        std::fs::remove_dir_all(d).unwrap();
    }
}
