use std::{collections::BTreeSet, env, fs};

const FOUNDATION_SOURCE: &str = "git+https://github.com/isarmg/sarmg-foundation-server.git?rev=";

fn locked_foundation_revision(lockfile: &str) -> String {
    let revisions = lockfile
        .lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix("source = \"")?.strip_suffix('"'))
        .filter_map(|source| source.strip_prefix(FOUNDATION_SOURCE))
        .map(|source| source.split_once('#').expect("locked Foundation source"))
        .map(|(requested, locked)| {
            assert_eq!(requested, locked, "Foundation revision must be immutable");
            assert!(
                locked.len() == 40 && locked.bytes().all(|byte| byte.is_ascii_hexdigit()),
                "Foundation revision must be full hexadecimal"
            );
            locked.to_owned()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        revisions.len(),
        1,
        "all Foundation crates must share one revision"
    );
    revisions
        .into_iter()
        .next()
        .expect("one Foundation revision")
}

fn main() {
    println!("cargo:rerun-if-env-changed=MEDIA_BACKUP_SOURCE_REVISION");
    let target = env::var("TARGET").expect("Cargo provides TARGET to build scripts");
    let revision =
        env::var("MEDIA_BACKUP_SOURCE_REVISION").unwrap_or_else(|_| "unversioned".to_owned());
    if revision != "unversioned"
        && (revision.len() != 40
            || !revision
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
    {
        panic!("MEDIA_BACKUP_SOURCE_REVISION must be 40 lowercase hexadecimal characters");
    }
    if target != "x86_64-unknown-linux-gnu" {
        panic!("all Media Backup server builds require x86_64-unknown-linux-gnu");
    }
    println!("cargo:rustc-env=MEDIA_BACKUP_SOURCE_REVISION={revision}");
    println!("cargo:rustc-env=MEDIA_BACKUP_BUILD_TARGET={target}");
    let foundation_revision = locked_foundation_revision(
        &fs::read_to_string("../../Cargo.lock").expect("read Cargo.lock"),
    );
    println!("cargo:rustc-env=SARMG_FOUNDATION_REVISION={foundation_revision}");
    println!("cargo:rerun-if-changed=../../Cargo.lock");
}
