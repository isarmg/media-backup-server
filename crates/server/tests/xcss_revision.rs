#[test]
fn embedded_xcss_revision_matches_every_locked_xcss_package() {
    let revision = env!("XCSS_REVISION");
    assert_eq!(revision.len(), 40);
    let lockfile = include_str!("../../../Cargo.lock");
    let sources = lockfile
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("source = \"git+https://github.com/isarmg/xcss.git?rev="))
        .collect::<Vec<_>>();
    assert!(!sources.is_empty());
    assert!(
        sources
            .iter()
            .all(|source| { source.contains(&format!("?rev={revision}#{revision}\"")) })
    );
}
