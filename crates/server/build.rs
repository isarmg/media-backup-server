use std::{collections::BTreeSet, env, fs, path::PathBuf};

const XCSS_SOURCE: &str = "git+https://github.com/isarmg/xcss.git?rev=";

fn locked_xcss_revision(lockfile: &str) -> String {
    let revisions = lockfile
        .lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix("source = \"")?.strip_suffix('"'))
        .filter_map(|source| source.strip_prefix(XCSS_SOURCE))
        .map(|source| source.split_once('#').expect("locked xcss source"))
        .map(|(requested, locked)| {
            assert_eq!(requested, locked, "xcss revision must be immutable");
            assert!(
                locked.len() == 40 && locked.bytes().all(|byte| byte.is_ascii_hexdigit()),
                "xcss revision must be full hexadecimal"
            );
            locked.to_owned()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        revisions.len(),
        1,
        "the xcss package must use one immutable revision"
    );
    revisions.into_iter().next().expect("one xcss revision")
}

fn main() {
    println!("cargo:rerun-if-env-changed=XCSS_WEB_DIST");
    println!("cargo:rerun-if-env-changed=XSZS_SOURCE_REVISION");
    let target = env::var("TARGET").expect("Cargo provides TARGET to build scripts");
    let revision = env::var("XSZS_SOURCE_REVISION").unwrap_or_else(|_| "unbound".to_owned());
    if revision != "unbound"
        && (revision.len() != 40
            || !revision
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
    {
        panic!("XSZS_SOURCE_REVISION must be 40 lowercase hexadecimal characters");
    }
    if target != "x86_64-unknown-linux-gnu" {
        panic!("all xszs server builds require x86_64-unknown-linux-gnu");
    }
    println!("cargo:rustc-env=XSZS_SOURCE_REVISION={revision}");
    println!("cargo:rustc-env=XSZS_BUILD_TARGET={target}");
    let xcss_revision =
        locked_xcss_revision(&fs::read_to_string("../../Cargo.lock").expect("read Cargo.lock"));
    println!("cargo:rustc-env=XCSS_REVISION={xcss_revision}");
    println!("cargo:rerun-if-changed=../../Cargo.lock");
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"));
    let web_root = env::var_os("XCSS_WEB_DIST")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            root.parent()
                .and_then(|directory| directory.parent())
                .expect("server crate within workspace")
                .join("web/dist")
        });
    xcss::web_assets::build::generate(&web_root).expect("build current embedded Web assets");
}
