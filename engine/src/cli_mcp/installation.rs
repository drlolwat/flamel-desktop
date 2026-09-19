//! Durable, content-addressed MCP executable copies for persistent CLI config.
//! AppImage mount paths disappear at exit; old verified copies remain usable.
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

fn digest(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub(super) fn durable_binary(source: &Path, home: &Path) -> Result<PathBuf, String> {
    let source = if source.is_absolute() {
        source.to_path_buf()
    } else {
        crate::provider_cli::resolve_executable(&source.to_string_lossy())
            .map(PathBuf::from)
            .ok_or("Bundled Flamel MCP executable is unavailable")?
    };
    copy_binary(&source, &home.join(".local/share/flamel/mcp"))
}

fn copy_binary(source: &Path, root: &Path) -> Result<PathBuf, String> {
    let hash =
        digest(source).map_err(|error| format!("Cannot read bundled MCP executable: {error}"))?;
    let release = root.join(&hash);
    std::fs::create_dir_all(&release).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
        std::fs::set_permissions(&release, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
    }
    let target = release.join(if cfg!(windows) {
        "flamel_mcp.exe"
    } else {
        "flamel_mcp"
    });
    if target.exists() {
        if digest(&target).map_err(|error| error.to_string())? == hash {
            return Ok(target);
        }
        return Err(
            "Installed MCP executable has changed; refusing to replace a running release".into(),
        );
    }
    let temporary = release.join(format!("staging-{}", uuid::Uuid::new_v4()));
    let copied = (|| -> std::io::Result<()> {
        let mut input = std::fs::File::open(source)?;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o700);
        }
        let mut output = options.open(&temporary)?;
        std::io::copy(&mut input, &mut output)?;
        output.flush()?;
        output.sync_all()?;
        if digest(&temporary)? != hash {
            return Err(std::io::Error::other("Bundled MCP changed while copying"));
        }
        std::fs::rename(&temporary, &target)?;
        Ok(())
    })();
    if let Err(error) = copied {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("Cannot install durable MCP executable: {error}"));
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn executable_survives_source_unmount_and_retains_previous_release() {
        let dir = std::env::temp_dir().join(format!("flamel-mcp-install-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("mounted-binary");
        std::fs::write(&source, b"first executable").unwrap();
        let first = copy_binary(&source, &dir.join("managed")).unwrap();
        std::fs::write(&source, b"second executable").unwrap();
        let second = copy_binary(&source, &dir.join("managed")).unwrap();
        std::fs::remove_file(source).unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read(first).unwrap(), b"first executable");
        assert_eq!(std::fs::read(second).unwrap(), b"second executable");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
