//! `build.json`, partitions and sandboxed path resolution, after `simulation/FS/FileHelper.java`.

use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct Partition {
    pub path: String,
    #[serde(default)]
    pub readonly: bool,
    #[serde(default)]
    pub hidden: bool,
    /// Negative means a built-in image layered under the real directory.
    #[serde(default)]
    pub source: i64,
}

/// A host directory the guest sees as one more partition on `drive0`.
#[derive(Clone, Debug, Deserialize)]
pub struct Share {
    pub name: String,
    pub path: PathBuf,
    #[serde(default)]
    pub readonly: bool,
}

impl Share {
    /// Parses the `--share` form `Name=/path[,ro]`.
    pub fn parse(spec: &str) -> Result<Share, String> {
        let Some((name, rest)) = spec.split_once('=') else {
            return Err(format!("--share {spec:?} is not Name=/path[,ro]"));
        };
        let (path, readonly) = match rest.strip_suffix(",ro") {
            Some(path) => (path, true),
            None => (rest, false),
        };
        if path.is_empty() {
            return Err(format!("--share {spec:?} names no directory"));
        }
        Ok(Share {
            name: name.to_string(),
            path: PathBuf::from(path),
            readonly,
        })
    }

    /// Checks the name and the directory, naming the share in anything it returns.
    fn check(&self, taken: &[Entry]) -> Result<(), String> {
        let named = |what: &str| format!("share {:?}: {what}", self.name);
        if self.name.is_empty() || !self.name.chars().all(|c| c.is_ascii_alphabetic()) {
            return Err(named("the name is not ASCII letters"));
        }
        if taken
            .iter()
            .any(|e| e.name.eq_ignore_ascii_case(&self.name))
        {
            return Err(named("the name is already a partition on this machine"));
        }
        if !self.path.is_dir() {
            return Err(named(&format!(
                "{} is not a directory",
                self.path.display()
            )));
        }
        Ok(())
    }
}

/// A partition as the drive reports it.
pub struct Entry {
    pub name: String,
    pub readonly: bool,
    pub hidden: bool,
}

#[derive(Debug, Deserialize)]
pub struct Build {
    pub entrypoint: String,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default)]
    pub partitions: Vec<Partition>,
}

fn default_language() -> String {
    "Lua".into()
}

#[derive(Debug)]
pub struct Disk {
    pub number: u32,
    pub root: PathBuf,
    pub uuid: String,
    pub build: Build,
    /// Host directories attached to this disk, each one more partition.
    pub shares: Vec<Share>,
}

/// What a path resolved to inside a partition.
pub struct Resolved {
    pub real: PathBuf,
    /// False when the partition is read-only.
    pub writable: bool,
}

#[derive(Debug)]
pub enum DiskError {
    NoBuildJson(PathBuf),
    BadBuildJson(String),
    Share(String),
}

impl std::fmt::Display for DiskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiskError::NoBuildJson(p) => write!(f, "no build.json at {}", p.display()),
            DiskError::BadBuildJson(e) => write!(f, "malformed build.json: {e}"),
            DiskError::Share(e) => write!(f, "{e}"),
        }
    }
}

impl Disk {
    pub fn load(root: &Path, number: u32) -> Result<Disk, DiskError> {
        let manifest = root.join("build.json");
        let text = std::fs::read_to_string(&manifest)
            .map_err(|_| DiskError::NoBuildJson(manifest.clone()))?;
        let build: Build =
            serde_json::from_str(&text).map_err(|e| DiskError::BadBuildJson(e.to_string()))?;
        Ok(Disk {
            number,
            root: root.to_path_buf(),
            uuid: uuid_for(number),
            build,
            shares: Vec::new(),
        })
    }

    /// Every partition on the disk, the shares after the ones `build.json` names.
    pub fn entries(&self) -> Vec<Entry> {
        let mut entries: Vec<Entry> = self
            .build
            .partitions
            .iter()
            .map(|p| Entry {
                name: p.path.clone(),
                readonly: p.readonly,
                hidden: p.hidden,
            })
            .collect();
        for share in &self.shares {
            entries.push(Entry {
                name: share.name.clone(),
                readonly: share.readonly,
                hidden: false,
            });
        }
        entries
    }

    pub fn entry(&self, name: &str) -> Option<Entry> {
        self.entries()
            .into_iter()
            .find(|e| e.name.eq_ignore_ascii_case(name))
    }

    /// Attaches a host directory as one more partition.
    pub fn attach_share(&mut self, share: &Share) -> Result<(), String> {
        share.check(&self.entries())?;
        self.shares.push(share.clone());
        Ok(())
    }

    /// The directory a partition lives in, and whether it may be written to.
    fn base(&self, name: &str) -> Option<(PathBuf, bool)> {
        if let Some(share) = self
            .shares
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
        {
            return Some((share.path.clone(), !share.readonly));
        }
        let partition = self
            .build
            .partitions
            .iter()
            .find(|p| p.path.eq_ignore_ascii_case(name))?;
        Some((self.root.join(&partition.path), !partition.readonly))
    }

    /// Splits `partition:/a/b` and resolves it, `None` where `FileHelper.getFile` gives a `NullFilepath`.
    pub fn resolve(&self, path: &str) -> Option<Resolved> {
        let (partition_name, tail) = split_path(path)?;
        let (base, writable) = self.base(&partition_name)?;
        let real = resolve_components(&base, &tail)?;
        if !contained_in(&base, &real) {
            return None;
        }
        Some(Resolved { real, writable })
    }

    /// The entrypoint as written in `build.json`.
    pub fn entrypoint(&self) -> &str {
        &self.build.entrypoint
    }

    pub fn language(&self) -> &str {
        &self.build.language
    }
}

/// A stable UUID derived from the disk's directory.
fn uuid_for(number: u32) -> String {
    format!("00000000-0000-4000-8000-{number:012x}")
}

/// Splits a path into its partition, everything before the last `:` as in `FileHelper.normalize`, and its components.
pub fn split_path(path: &str) -> Option<(String, Vec<String>)> {
    let mut sections: Vec<&str> = path.split(':').collect();
    let tail = sections.pop()?;
    let partition = sections.first()?.to_string();

    // `validatePathStatic`: the address is letters only.
    if partition.is_empty() || !partition.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }

    let components: Vec<String> = tail
        .split(['/', '\\'])
        .filter(|c| !c.is_empty())
        .map(str::to_string)
        .collect();

    // The regex rejects `.`, `..` and any component ending in a dot.
    if components.iter().any(|c| {
        c.ends_with('.')
            || !c
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | ' ' | '.'))
    }) {
        return None;
    }

    Some((partition, components))
}

/// Walks the components case-insensitively, one directory at a time.
fn resolve_components(base: &Path, components: &[String]) -> Option<PathBuf> {
    let mut current = base.to_path_buf();
    for (i, component) in components.iter().enumerate() {
        let exact = current.join(component);
        if exact.exists() {
            current = exact;
            continue;
        }
        match case_insensitive_child(&current, component) {
            Some(found) => current = found,
            // A missing leaf is fine — the caller may be creating it.
            None if i + 1 == components.len() => current = exact,
            None => return None,
        }
    }
    Some(current)
}

fn case_insensitive_child(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().find_map(|entry| {
        let file_name = entry.file_name();
        file_name
            .to_str()
            .is_some_and(|n| n.eq_ignore_ascii_case(name))
            .then(|| entry.path())
    })
}

/// Rejects a path that escaped the partition, e.g. through a symlink.
fn contained_in(base: &Path, candidate: &Path) -> bool {
    let root = match base.canonicalize() {
        Ok(p) => p,
        Err(_) => return false,
    };
    // A leaf that does not exist yet canonicalizes through its parent.
    let resolved = candidate.canonicalize().or_else(|_| {
        let parent = candidate.parent().ok_or(())?;
        let name = candidate.file_name().ok_or(())?;
        parent.canonicalize().map(|p| p.join(name)).map_err(|_| ())
    });
    resolved.is_ok_and(|p| p.starts_with(&root))
}

/// The attached disks, addressed by lowest free slot as in `DiskManager`.
pub struct DiskSet {
    disks: Vec<Disk>,
}

impl DiskSet {
    /// No disks attached, for tests that exercise the interpreter alone.
    pub fn empty() -> DiskSet {
        DiskSet { disks: Vec::new() }
    }

    /// Attaches `<root>/<boot>` in slot 0; the drive bay would add the rest.
    pub fn load(root: &Path, boot: u32) -> Result<DiskSet, DiskError> {
        Ok(DiskSet {
            disks: vec![Disk::load(&root.join(boot.to_string()), boot)?],
        })
    }

    /// Attaches a share to the boot disk, which is the one `drive0` addresses.
    pub fn attach_share(&mut self, share: &Share) -> Result<(), DiskError> {
        match self.disks.first_mut() {
            Some(disk) => disk.attach_share(share).map_err(DiskError::Share),
            None => Err(DiskError::Share(format!(
                "share {:?}: the machine has no disk",
                share.name
            ))),
        }
    }

    /// A nil id means disk 0, the boot disk.
    pub fn get(&self, index: Option<i64>) -> Option<&Disk> {
        self.disks.get(usize::try_from(index.unwrap_or(0)).ok()?)
    }

    pub fn boot(&self) -> &Disk {
        &self.disks[0]
    }

    pub fn has_boot(&self) -> bool {
        !self.disks.is_empty()
    }

    pub fn len(&self) -> usize {
        self.disks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.disks.is_empty()
    }

    /// `files.getDisks`: slot numbers, not the save's directory names.
    pub fn indices(&self) -> Vec<i64> {
        (0..self.disks.len() as i64).collect()
    }
}
