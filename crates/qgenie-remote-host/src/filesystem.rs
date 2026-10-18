//! Directory capabilities bound to opened root inodes. Remote paths never reach
//! a canonicalize-then-open sequence. Existing session actions do not use roots.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use qgenie_remote_proto::{Counter, wire::DirectoryEntry};
use qgenie_remote_transport::ipc::current_uid;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    ffi::{CStr, CString},
    fs::File,
    os::{
        fd::{AsRawFd, FromRawFd, IntoRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::Semaphore;

#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    #[error("invalid or unsupported path/cursor")]
    Invalid,
    #[error("path is outside allowed roots, unsafe, or a symlink")]
    Denied,
    #[error("directory unavailable")]
    Unavailable,
    #[error("directory or worker capacity exceeded")]
    Capacity,
    #[error("directory changed; restart browsing without a cursor")]
    Changed,
}
pub struct Page {
    pub entries: Vec<DirectoryEntry>,
    pub next: Option<String>,
    pub skipped_unsupported: u16,
}
pub struct Directory {
    file: File,
    logical: PathBuf,
}
impl Directory {
    pub fn logical_path(&self) -> &Path {
        &self.logical
    }
    pub fn file(&self) -> &File {
        &self.file
    }
    pub fn into_file(self) -> File {
        self.file
    }
    pub fn list(&self, cursor: Option<&str>, limit: u16) -> Result<Page, Error> {
        if !(1..=256).contains(&limit) {
            return Err(Error::Invalid);
        }
        let before = Stamp::read(&self.file)?;
        let cursor = cursor
            .map(|text| {
                if text.len() > 2048 {
                    return Err(Error::Invalid);
                }
                let bytes = URL_SAFE_NO_PAD.decode(text).map_err(|_| Error::Invalid)?;
                qgenie_remote_proto::decode_json::<Cursor>(&bytes).map_err(|_| Error::Invalid)
            })
            .transpose()?;
        if let Some(cursor) = &cursor {
            if cursor.stamp != before {
                return Err(Error::Changed);
            }
            if !name(&cursor.last) {
                return Err(Error::Invalid);
            }
        }
        // Opening "." gives an independent directory offset. dup/try_clone would
        // share offsets and race concurrent enumerations of the same root.
        let copy = open_at(&self.file, c".")?;
        let raw = copy.into_raw_fd();
        // SAFETY: raw owns a directory descriptor and fdopendir takes ownership on success.
        let pointer = unsafe { libc::fdopendir(raw) };
        if pointer.is_null() {
            // SAFETY: failed fdopendir leaves the descriptor owned here.
            drop(unsafe { File::from_raw_fd(raw) });
            return Err(Error::Unavailable);
        }
        struct Stream(*mut libc::DIR);
        impl Drop for Stream {
            fn drop(&mut self) {
                // SAFETY: this guard exclusively owns the DIR pointer.
                unsafe { libc::closedir(self.0) };
            }
        }
        let stream = Stream(pointer);
        let mut entries = Vec::new();
        let mut count = 0;
        let mut skipped = 0u16;
        loop {
            errno(0);
            // SAFETY: stream owns a live directory; its entry is consumed before the next call.
            let entry = unsafe { libc::readdir(stream.0) };
            if entry.is_null() {
                if last_errno() != 0 {
                    return Err(Error::Unavailable);
                }
                break;
            }
            count += 1;
            if count > 8194 {
                return Err(Error::Capacity);
            }
            // SAFETY: POSIX dirent has a NUL-terminated name, valid until the next readdir.
            let bytes = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
            if bytes == b"." || bytes == b".." {
                continue;
            }
            let Ok(text) = std::str::from_utf8(bytes) else {
                skipped = skipped.saturating_add(1);
                continue;
            };
            if !name(text) {
                skipped = skipped.saturating_add(1);
                continue;
            }
            let encoded = CString::new(bytes).map_err(|_| Error::Invalid)?;
            // Classification is a hint only. Every later browse/cwd resolve opens
            // with O_DIRECTORY|O_NOFOLLOW and rechecks ownership on the descriptor.
            let directory = open_at(&self.file, &encoded).is_ok();
            entries.push((text.to_owned(), directory));
        }
        if Stamp::read(&self.file)? != before {
            return Err(Error::Changed);
        }
        entries.sort_unstable_by(|left, right| left.0.cmp(&right.0));
        let digest = Sha256::digest(serde_json::to_vec(&entries).map_err(|_| Error::Invalid)?);
        let digest = digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if cursor
            .as_ref()
            .is_some_and(|cursor| cursor.digest != digest)
        {
            return Err(Error::Changed);
        }
        let mut page = Vec::new();
        let mut size = 0;
        let mut more = false;
        for (name, directory) in entries
            .into_iter()
            .filter(|(name, _)| cursor.as_ref().is_none_or(|cursor| name > &cursor.last))
        {
            let path = self
                .logical
                .join(&name)
                .into_os_string()
                .into_string()
                .map_err(|_| Error::Invalid)?;
            if path.len() > 4096 {
                skipped = skipped.saturating_add(1);
                continue;
            }
            let entry = DirectoryEntry {
                name,
                path,
                directory,
            };
            let bytes = serde_json::to_vec(&entry)
                .map_err(|_| Error::Invalid)?
                .len();
            if page.len() >= usize::from(limit) || size + bytes > 512 * 1024 {
                more = true;
                break;
            }
            size += bytes;
            page.push(entry);
        }
        let next = if more {
            let last = page.last().ok_or(Error::Capacity)?.name.clone();
            Some(
                URL_SAFE_NO_PAD.encode(
                    serde_json::to_vec(&Cursor {
                        stamp: before,
                        digest,
                        last,
                    })
                    .map_err(|_| Error::Invalid)?,
                ),
            )
        } else {
            None
        };
        Ok(Page {
            entries: page,
            next,
            skipped_unsupported: skipped,
        })
    }
}
struct Root {
    path: PathBuf,
    alias: PathBuf,
    file: File,
}
pub struct Roots {
    roots: Vec<Root>,
}
impl Roots {
    pub fn open(paths: &[PathBuf]) -> Result<Self, Error> {
        if paths.is_empty() || paths.len() > 16 {
            return Err(Error::Invalid);
        }
        let mut roots = Vec::new();
        for path in paths {
            validate(path)?;
            for ancestor in path.ancestors() {
                let metadata = std::fs::symlink_metadata(ancestor).map_err(|_| Error::Denied)?;
                if (metadata.uid() != current_uid() && metadata.uid() != 0)
                    || (!metadata.file_type().is_symlink()
                        && metadata.mode() & 0o022 != 0
                        && !(metadata.uid() == 0 && metadata.mode() & 0o1000 != 0))
                {
                    return Err(Error::Denied);
                }
            }
            let canonical = std::fs::canonicalize(path).map_err(|_| Error::Denied)?;
            validate(&canonical)?;
            let mut file = open_root()?;
            for component in canonical.components().skip(1) {
                let text =
                    CString::new(component.as_os_str().as_bytes()).map_err(|_| Error::Invalid)?;
                file = open_at(&file, &text)?;
            }
            roots.push(Root {
                path: canonical,
                alias: path.clone(),
                file,
            });
        }
        roots.sort_by_key(|root| std::cmp::Reverse(root.path.components().count()));
        Ok(Self { roots })
    }
    pub fn paths(&self) -> Vec<PathBuf> {
        self.roots.iter().map(|root| root.path.clone()).collect()
    }
    pub fn resolve(&self, path: &Path) -> Result<Directory, Error> {
        validate(path)?;
        let (root, relative) = self
            .roots
            .iter()
            .find_map(|root| {
                path.strip_prefix(&root.path)
                    .or_else(|_| path.strip_prefix(&root.alias))
                    .ok()
                    .map(|relative| (root, relative))
            })
            .ok_or(Error::Denied)?;
        let mut file = open_at(&root.file, c".")?;
        let mut logical = root.path.clone();
        for component in relative.components() {
            logical.push(component);
            let text =
                CString::new(component.as_os_str().as_bytes()).map_err(|_| Error::Invalid)?;
            file = open_at(&file, &text)?;
        }
        Ok(Directory { file, logical })
    }
}
#[derive(Clone)]
pub struct Filesystem {
    roots: Arc<Roots>,
    workers: Arc<Semaphore>,
}
impl Filesystem {
    pub fn new(roots: Roots) -> Self {
        Self {
            roots: Arc::new(roots),
            workers: Arc::new(Semaphore::new(4)),
        }
    }
    pub fn roots(&self) -> &Roots {
        &self.roots
    }
    pub async fn list(
        &self,
        path: String,
        cursor: Option<String>,
        limit: u16,
    ) -> Result<Page, Error> {
        validate(Path::new(&path))?;
        if cursor.as_ref().is_some_and(|cursor| cursor.len() > 2048) {
            return Err(Error::Invalid);
        }
        let permit = self
            .workers
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Capacity)?;
        let roots = self.roots.clone();
        let task = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            roots
                .resolve(Path::new(&path))?
                .list(cursor.as_deref(), limit)
        });
        tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .map_err(|_| Error::Unavailable)?
            .map_err(|_| Error::Unavailable)?
    }
}
fn name(text: &str) -> bool {
    !text.is_empty()
        && text != "."
        && text != ".."
        && text.len() <= 255
        && !text.contains('/')
        && !text.chars().any(char::is_control)
}
fn validate(path: &Path) -> Result<(), Error> {
    let text = path.to_str().ok_or(Error::Invalid)?;
    if !path.is_absolute()
        || text.len() > 4096
        || text.chars().any(char::is_control)
        || text.split('/').any(|part| part == "." || part == "..")
    {
        return Err(Error::Invalid);
    }
    Ok(())
}
fn check(file: &File) -> Result<(), Error> {
    let metadata = file.metadata().map_err(|_| Error::Unavailable)?;
    if !metadata.is_dir()
        || (metadata.uid() != current_uid() && metadata.uid() != 0)
        || (metadata.mode() & 0o022 != 0 && !(metadata.uid() == 0 && metadata.mode() & 0o1000 != 0))
    {
        return Err(Error::Denied);
    }
    Ok(())
}
fn open_root() -> Result<File, Error> {
    // SAFETY: a static absolute path and valid open flags are supplied.
    let raw = unsafe {
        libc::open(
            c"/".as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if raw < 0 {
        return Err(Error::Unavailable);
    } // SAFETY: this function exclusively owns the newly opened descriptor.
    let file = unsafe { File::from_raw_fd(raw) };
    check(&file)?;
    Ok(file)
}
fn open_at(parent: &File, name: &CStr) -> Result<File, Error> {
    // SAFETY: parent is live, name is NUL terminated, and only directories are opened.
    let raw = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY
                | libc::O_DIRECTORY
                | libc::O_CLOEXEC
                | libc::O_NOFOLLOW
                | libc::O_NONBLOCK,
        )
    };
    if raw < 0 {
        return Err(Error::Denied);
    } // SAFETY: raw is a newly owned descriptor and is wrapped exactly once.
    let file = unsafe { File::from_raw_fd(raw) };
    check(&file)?;
    Ok(file)
}
#[derive(Serialize, Deserialize, PartialEq, Eq)]
struct Stamp {
    device: Counter,
    inode: Counter,
    seconds: i64,
    nanos: i64,
}
impl Stamp {
    fn read(file: &File) -> Result<Self, Error> {
        check(file)?;
        let metadata = file.metadata().map_err(|_| Error::Unavailable)?;
        Ok(Self {
            device: Counter(metadata.dev()),
            inode: Counter(metadata.ino()),
            seconds: metadata.mtime(),
            nanos: metadata.mtime_nsec(),
        })
    }
}
#[derive(Serialize, Deserialize)]
struct Cursor {
    stamp: Stamp,
    digest: String,
    last: String,
}
#[cfg(target_os = "linux")]
fn errno(value: i32) {
    // SAFETY: errno is a thread-local pointer valid for this thread.
    unsafe {
        *libc::__errno_location() = value;
    }
}
#[cfg(target_os = "linux")]
fn last_errno() -> i32 {
    // SAFETY: errno is a thread-local pointer valid for this thread.
    unsafe { *libc::__errno_location() }
}
#[cfg(target_os = "macos")]
fn errno(value: i32) {
    // SAFETY: errno is a thread-local pointer valid for this thread.
    unsafe {
        *libc::__error() = value;
    }
}
#[cfg(target_os = "macos")]
fn last_errno() -> i32 {
    // SAFETY: errno is a thread-local pointer valid for this thread.
    unsafe { *libc::__error() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    #[test]
    fn rooted_handles_reject_traversal_symlinks_and_substitution_and_page_stably() {
        let directory = tempfile::Builder::new()
            .permissions(
                <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
            )
            .tempdir()
            .unwrap();
        let root = directory.path().join("root");
        let outside = directory.path().join("outside");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("private"), b"outside").unwrap();
        std::fs::create_dir(root.join("child")).unwrap();
        std::fs::write(root.join("child/safe"), b"inside").unwrap();
        for index in 0..20 {
            std::fs::write(root.join(format!("file-{index:02}")), b"fixture").unwrap();
        }
        let roots = Roots::open(std::slice::from_ref(&root)).unwrap();
        assert!(roots.resolve(&outside).is_err());
        assert!(roots.resolve(&root.join("../outside")).is_err());
        symlink(&outside, root.join("escape")).unwrap();
        assert!(roots.resolve(&root.join("escape")).is_err());
        let held = roots.resolve(&root.join("child")).unwrap();
        std::fs::rename(root.join("child"), root.join("moved")).unwrap();
        symlink(&outside, root.join("child")).unwrap();
        assert!(roots.resolve(&root.join("child")).is_err());
        let page = held.list(None, 10).unwrap();
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.entries[0].name, "safe");
        let folder = roots.resolve(&root).unwrap();
        let first = folder.list(None, 5).unwrap();
        let again = folder.list(None, 5).unwrap();
        assert_eq!(
            first
                .entries
                .iter()
                .map(|entry| &entry.name)
                .collect::<Vec<_>>(),
            again
                .entries
                .iter()
                .map(|entry| &entry.name)
                .collect::<Vec<_>>()
        );
        let second = folder.list(first.next.as_deref(), 5).unwrap();
        assert!(first.entries.last().unwrap().name < second.entries.first().unwrap().name);
        assert!(matches!(
            held.list(first.next.as_deref(), 5),
            Err(Error::Changed)
        ));
        let modified = folder.file().metadata().unwrap().modified().unwrap();
        std::fs::write(root.join("new-entry"), b"changed").unwrap();
        folder
            .file()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        assert!(matches!(
            folder.list(first.next.as_deref(), 5),
            Err(Error::Changed)
        ));
        std::fs::set_permissions(root.join("moved"), std::fs::Permissions::from_mode(0o777))
            .unwrap();
        assert!(roots.resolve(&root.join("moved")).is_err());
        assert!(held.list(None, 10).is_err());
    }
}
