use std::env;

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
}
