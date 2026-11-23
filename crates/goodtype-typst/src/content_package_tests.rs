use super::*;

#[test]
fn content_packages_resolve_offline_without_masking_the_users_cache() {
    let user = tempfile::tempdir().unwrap();
    let pack = tempfile::tempdir().unwrap();
    let version = "preview/zap/0.4.0";
    let spec = "@preview/zap:0.4.0".parse().unwrap();
    let manifest = VirtualPath::new("typst.toml").unwrap();
    let packages = SystemPackages::from_parts(
        None,
        Some(FsPackages::new(user.path())),
        UniversePackages::new(OfflineDownloader),
    );
    let content = FsPackages::new(pack.path());
    let read = |scope: Option<&[String]>| {
        let root = content_package(&packages, Some(&content), scope, &spec).unwrap();
        std::fs::read_to_string(root.resolve(&manifest).unwrap()).unwrap()
    };
    std::fs::create_dir_all(pack.path().join(version)).unwrap();
    std::fs::write(pack.path().join(version).join("typst.toml"), "pack").unwrap();
    assert_eq!(read(Some(&[version.into()])), "pack");
    assert!(content_package(&packages, Some(&content), Some(&[]), &spec).is_err());
    std::fs::create_dir_all(user.path().join(version)).unwrap();
    std::fs::write(user.path().join(version).join("typst.toml"), "user").unwrap();
    assert_eq!(read(None), "user");
    assert_eq!(
        std::fs::read_to_string(pack.path().join(version).join("typst.toml")).unwrap(),
        "pack"
    );
}
