//! Native cwd policy is checked independently of the connector. Retain the
//! resolved inode through fork/exec; a later rename/symlink never reopens a path.
use qgenie_remote_transport::ipc::current_uid;
use std::{
    ffi::{CStr, CString},
    fs::File,
    io,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Path, PathBuf},
};

struct Root {
    path: PathBuf,
    alias: PathBuf,
    file: File,
}
pub(crate) struct Roots {
    roots: Vec<Root>,
}
pub(crate) struct Directory {
    pub path: PathBuf,
    pub file: File,
}
impl Roots {
    pub fn open(paths: &[PathBuf]) -> io::Result<Self> {
        if paths.is_empty() || paths.len() > 16 {
            return Err(denied());
        }
        let mut roots = Vec::new();
        for path in paths {
            validate(path)?;
            for ancestor in path.ancestors() {
                let metadata = std::fs::symlink_metadata(ancestor)?;
                if (metadata.uid() != 0 && metadata.uid() != current_uid())
                    || (!metadata.file_type().is_symlink()
                        && unsafe_mode(metadata.uid(), metadata.mode()))
                {
                    return Err(denied());
                }
            }
            let canonical = std::fs::canonicalize(path)?;
            validate(&canonical)?;
            // SAFETY: a fixed NUL-terminated path and valid directory flags.
            let raw = unsafe {
                libc::open(
                    c"/".as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if raw < 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: this function exclusively owns the new descriptor.
            let mut file = unsafe { File::from_raw_fd(raw) };
            check(&file)?;
            for component in canonical.components().skip(1) {
                let name = CString::new(component.as_os_str().as_bytes()).map_err(|_| denied())?;
                file = open_at(&file, &name)?;
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
    pub fn resolve(&self, path: &Path) -> io::Result<Directory> {
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
            .ok_or_else(denied)?;
        let mut file = open_at(&root.file, c".")?;
        let mut logical = root.path.clone();
        for component in relative.components() {
            logical.push(component);
            let name = CString::new(component.as_os_str().as_bytes()).map_err(|_| denied())?;
            file = open_at(&file, &name)?;
        }
        Ok(Directory {
            path: logical,
            file,
        })
    }
}
fn validate(path: &Path) -> io::Result<()> {
    let text = path.to_str().ok_or_else(denied)?;
    if !path.is_absolute()
        || text.len() > 4096
        || text.chars().any(char::is_control)
        || text.split('/').any(|part| part == "." || part == "..")
    {
        return Err(denied());
    }
    Ok(())
}
fn unsafe_mode(uid: u32, mode: u32) -> bool {
    mode & 0o022 != 0 && !(uid == 0 && mode & 0o1000 != 0)
}
fn check(file: &File) -> io::Result<()> {
    let metadata = file.metadata()?;
    if !metadata.is_dir()
        || (metadata.uid() != current_uid() && metadata.uid() != 0)
        || unsafe_mode(metadata.uid(), metadata.mode())
    {
        return Err(denied());
    }
    Ok(())
}
fn open_at(parent: &File, name: &CStr) -> io::Result<File> {
    // SAFETY: parent remains live and name is NUL terminated. Never follow a symlink.
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
        return Err(io::Error::last_os_error());
    }
    // SAFETY: wrap the new owned descriptor exactly once.
    let file = unsafe { File::from_raw_fd(raw) };
    check(&file)?;
    Ok(file)
}
fn denied() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "native cwd is outside allowed roots or unsafe",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_policy_rejects_symlinks_traversal_and_changed_permissions() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let temp = tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let path = std::fs::canonicalize(temp.path()).unwrap();
        let child = path.join("child");
        std::fs::create_dir(&child).unwrap();
        let roots = Roots::open(std::slice::from_ref(&path)).unwrap();
        let directory = roots.resolve(&child).unwrap();
        assert!(roots.resolve(&child.join("..")).is_err());
        assert!(roots.resolve(Path::new("/etc")).is_err());
        std::fs::rename(&child, path.join("held")).unwrap();
        symlink("/etc", &child).unwrap();
        assert!(roots.resolve(&child).is_err());
        assert_eq!(
            directory.file.metadata().unwrap().ino(),
            std::fs::metadata(path.join("held")).unwrap().ino()
        );
        std::fs::set_permissions(path.join("held"), std::fs::Permissions::from_mode(0o777))
            .unwrap();
        assert!(roots.resolve(&path.join("held")).is_err());
    }
}
