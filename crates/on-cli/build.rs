use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

include!("src/seal.rs");

fn main() {
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let model = std::env::var("OPENNOTE_MODEL")
        .map(PathBuf::from)
        .unwrap_or_else(|_| manifest.join("../../models/helios-beta1.5.json"));
    println!("cargo:rerun-if-env-changed=OPENNOTE_MODEL");
    let bytes = match std::fs::read(&model) {
        Ok(bytes) => {
            println!("cargo:rerun-if-changed={}", model.display());
            bytes
        }
        Err(_) => {
            println!("cargo:rerun-if-changed=build.rs");
            Vec::new()
        }
    };
    let key = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() as u64 ^ 0x5DEE_CE66_D1CE_4E5B;
    std::fs::write(out.join("model.bin"), seal(&bytes, key)).unwrap();
    std::fs::write(out.join("model_key.rs"), format!("{key}u64")).unwrap();
    let release = std::env::var("OPENNOTE_RELEASE_KEY")
        .ok()
        .or_else(|| std::fs::read_to_string(manifest.join("../../models/release-key.txt")).ok())
        .and_then(|k| k.trim().parse::<u64>().ok());
    println!("cargo:rerun-if-env-changed=OPENNOTE_RELEASE_KEY");
    println!("cargo:rerun-if-changed={}", manifest.join("../../models/release-key.txt").display());
    std::fs::write(out.join("release_key.rs"), match release {
        Some(k) => format!("Some({k}u64)"),
        None => "None".to_string(),
    })
    .unwrap();
    let name = model.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    println!("cargo:rustc-env=OPENNOTE_MODEL_NAME={}", if bytes.is_empty() { String::new() } else { name });
}
