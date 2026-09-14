#[test]
fn embedded_foundation_revision_matches_every_locked_foundation_package() {
    let revision = env!("SARMG_FOUNDATION_REVISION");
    assert_eq!(revision.len(), 40);
    let lockfile = include_str!("../../../Cargo.lock");
    let sources = lockfile
        .lines()
        .map(str::trim)
        .filter(|line| {
            line.starts_with(
                "source = \"git+https://github.com/isarmg/sarmg-foundation-server.git?rev=",
            )
        })
        .collect::<Vec<_>>();
    assert!(!sources.is_empty());
    assert!(sources
        .iter()
        .all(|source| { source.contains(&format!("?rev={revision}#{revision}\"")) }));
}
