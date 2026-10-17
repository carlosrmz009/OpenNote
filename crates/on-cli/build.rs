use std::path::PathBuf;

include!("src/seal.rs");

fn main() {
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let model = std::env::var("OPENNOTE_MODEL")
        .map(PathBuf::from)
        .unwrap_or_else(|_| manifest.join("../../models/helios-beta1.8.json"));
    println!("cargo:rerun-if-env-changed=OPENNOTE_MODEL");
    let bytes = match std::fs::read(&model) {
        Ok(bytes) => {
            println!("cargo:rerun-if-changed={}", model.display());
            bytes
        }
        Err(_) => {
            // Cargo treats a missing file as always changed, which would rebuild this crate
            // every time; the folder it would go in is watched instead, when there is one.
            match model.parent().filter(|folder| folder.is_dir()) {
                Some(folder) => println!("cargo:rerun-if-changed={}", folder.display()),
                None => println!("cargo:rerun-if-changed=build.rs"),
            }
            println!(
                "cargo:warning=no model at {}, so this build fingers with the rules alone \
                 (set OPENNOTE_MODEL, or add the file and run `cargo clean -p on-cli`)",
                model.display()
            );
            Vec::new()
        }
    };
    // Derived from the model, so the same model always seals the same way and two builds
    // of the same commit are identical.
    let key = fnv(&bytes) ^ 0x5DEE_CE66_D1CE_4E5B;
    std::fs::write(out.join("model.bin"), seal(&bytes, key)).unwrap();
    std::fs::write(out.join("model_key.rs"), format!("{key}u64")).unwrap();
    let release = std::env::var("OPENNOTE_RELEASE_KEY")
        .ok()
        .or_else(|| std::fs::read_to_string(manifest.join("../../models/release-key.txt")).ok())
        .and_then(|k| match k.trim().parse::<u64>() {
            Ok(key) => Some(key),
            Err(_) => {
                println!(
                    "cargo:warning=the release key is not a number, so this build cannot open published models"
                );
                None
            }
        });
    println!("cargo:rerun-if-env-changed=OPENNOTE_RELEASE_KEY");
    let release_file = manifest.join("../../models/release-key.txt");
    if release_file.exists() {
        println!("cargo:rerun-if-changed={}", release_file.display());
    }
    std::fs::write(
        out.join("release_key.rs"),
        match release {
            Some(k) => format!("Some({k}u64)"),
            None => "None".to_string(),
        },
    )
    .unwrap();
    let name = model.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    println!("cargo:rustc-env=OPENNOTE_MODEL_NAME={}", if bytes.is_empty() { String::new() } else { name });
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .fold(0xcbf2_9ce4_8422_2325, |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3))
}
