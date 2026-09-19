//! Publish a complete vault key once, even when multiple runtimes start together.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::{fs, io, io::Write, path::Path};

fn read(path: &Path) -> io::Result<String> {
    let value = fs::read_to_string(path)?;
    if value.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "vault key is empty",
        ));
    }
    Ok(value)
}

pub(super) fn load_or_create(path: &Path) -> Result<String, String> {
    match read(path) {
        Ok(value) => return Ok(value),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("Could not read credential vault key: {error}")),
    }
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut directory = fs::DirBuilder::new();
    directory.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        directory.mode(0o700);
    }
    // Do not chmod an existing parent: an explicitly configured key may live in
    // a shared system directory. Only newly created directories are private.
    directory
        .create(parent)
        .map_err(|error| format!("Could not create vault directory: {error}"))?;
    let staged = parent.join(format!(".vault-key-{}", uuid::Uuid::new_v4()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&staged)
        .map_err(|error| format!("Could not stage credential vault key: {error}"))?;
    let value = STANDARD.encode(rand::random::<[u8; 32]>());
    let written = file
        .write_all(value.as_bytes())
        .and_then(|()| file.sync_all());
    drop(file);
    let result = written.and_then(|()| {
        // A same-directory hard link publishes the fully written inode without
        // ever replacing an existing key. A losing process reads the winner.
        // Unsupported filesystems fail closed instead of using a racy overwrite.
        match fs::hard_link(&staged, path) {
            Ok(()) => Ok(value),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => read(path),
            Err(error) => Err(error),
        }
    });
    let _ = fs::remove_file(&staged);
    #[cfg(unix)]
    if result.is_ok() {
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("Could not sync credential vault directory: {error}"))?;
    }
    result.map_err(|error| format!("Could not initialize credential vault key: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_initialization_publishes_one_complete_key() {
        let root = std::env::temp_dir().join(format!("flamel-vault-race-{}", uuid::Uuid::new_v4()));
        let path = root.join("vault.key");
        let barrier = std::sync::Barrier::new(24);
        let values = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..24)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        load_or_create(&path).unwrap()
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>()
        });
        let stored = fs::read_to_string(&path).unwrap();
        assert_eq!(STANDARD.decode(&stored).unwrap().len(), 32);
        assert!(values.iter().all(|value| value == &stored));
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(&root).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn existing_key_is_never_changed_and_empty_keys_fail_closed() {
        let root =
            std::env::temp_dir().join(format!("flamel-vault-existing-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let path = root.join("vault.key");
        let original = "existing legacy key with preserved trailing newline\n";
        fs::write(&path, original).unwrap();
        assert_eq!(load_or_create(&path).unwrap(), original);
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        fs::write(&path, "").unwrap();
        assert!(load_or_create(&path).unwrap_err().contains("empty"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn creating_key_does_not_change_existing_parent_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let root =
            std::env::temp_dir().join(format!("flamel-vault-parent-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
        load_or_create(&root.join("vault.key")).unwrap();
        assert_eq!(
            fs::metadata(&root).unwrap().permissions().mode() & 0o777,
            0o755
        );
        fs::remove_dir_all(root).unwrap();
    }
}
