use std::path::Path;

pub(crate) fn normalize(path: &Path) -> String {
    let path = path.to_string_lossy();
    #[cfg(windows)]
    {
        if let Some(path) = path.strip_prefix(r"\\?\UNC\") {
            return format!("//{path}").replace('\\', "/");
        }
        if let Some(path) = path.strip_prefix(r"\\?\")
            && path.as_bytes().get(1) == Some(&b':')
        {
            return path.replace('\\', "/");
        }
        path.replace('\\', "/")
    }
    #[cfg(not(windows))]
    {
        path.into_owned()
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn preserves_absolute_drive_and_unc_paths() {
        assert_eq!(normalize(Path::new(r"\\?\C:\repo")), "C:/repo");
        assert_eq!(normalize(Path::new(r"C:\repo")), "C:/repo");
        assert_eq!(
            normalize(Path::new(r"\\?\UNC\server\share\repo")),
            "//server/share/repo"
        );
        assert_eq!(
            normalize(Path::new(r"\\server\share\repo")),
            "//server/share/repo"
        );
        assert_eq!(
            normalize(Path::new(r"\\?\Volume{guid}\repo")),
            "//?/Volume{guid}/repo"
        );
    }
}
