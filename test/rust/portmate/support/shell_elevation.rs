#[cfg(test)]
pub(super) fn test_resource_lease(directory: tempfile::TempDir) -> ElevationResources {
    let file = fs::File::create(directory.path().join("lease")).unwrap();
    let pin = BoundDirectory::open(directory.path()).unwrap();
    ElevationResources {
        _file: file,
        _directory_pin: pin,
        _directory: directory,
    }
}
