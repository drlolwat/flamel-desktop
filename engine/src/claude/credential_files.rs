use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub(crate) struct CredentialMaterializationError {
    operation: &'static str,
    path: Option<PathBuf>,
    detail: String,
}

impl CredentialMaterializationError {
    pub(crate) fn new(
        operation: &'static str,
        path: Option<&Path>,
        error: impl fmt::Display,
    ) -> Self {
        Self {
            operation,
            path: path.map(Path::to_path_buf),
            detail: error.to_string(),
        }
    }
}

impl fmt::Display for CredentialMaterializationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(path) = &self.path {
            write!(
                formatter,
                "{} at {}: {}",
                self.operation,
                path.display(),
                self.detail
            )
        } else {
            write!(formatter, "{}: {}", self.operation, self.detail)
        }
    }
}

impl std::error::Error for CredentialMaterializationError {}

pub(crate) fn ensure_private_directory(path: &Path) -> Result<(), CredentialMaterializationError> {
    fs::create_dir_all(path).map_err(|error| {
        CredentialMaterializationError::new(
            "failed to create credential directory",
            Some(path),
            error,
        )
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|error| {
            CredentialMaterializationError::new(
                "failed to secure credential directory",
                Some(path),
                error,
            )
        })?;
    }

    Ok(())
}

pub(crate) fn write_private_file(
    path: &Path,
    contents: &[u8],
) -> Result<(), CredentialMaterializationError> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }

    let mut file = options.open(path).map_err(|error| {
        CredentialMaterializationError::new("failed to open credential file", Some(path), error)
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|error| {
                CredentialMaterializationError::new(
                    "failed to secure credential file",
                    Some(path),
                    error,
                )
            })?;
    }

    file.write_all(contents).map_err(|error| {
        CredentialMaterializationError::new("failed to write credential file", Some(path), error)
    })
}
