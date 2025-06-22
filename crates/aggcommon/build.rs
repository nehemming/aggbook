use std::{env, fs, path::PathBuf, process::Command};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let diagnostic = env::var("PROTO_BUILD_DIAGNOSTIC").unwrap_or_default();
    let enable_diag = diagnostic == "1" || diagnostic.eq_ignore_ascii_case("true");

    let dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let proto = dir.join("proto/orderbook.proto");
    let out_dir = dir.join("src/proto");

    // Create the output directory if it doesn't exist
    if !out_dir.exists() {
        if enable_diag {
            println!("cargo:warning=Output directory does not exist. Creating it...");
        }
        fs::create_dir_all(&out_dir)?;
    }

    if enable_diag {
        // Try to find the protoc binary via PATH
        match Command::new("which").arg("protoc").output() {
            Ok(output) if output.status.success() => {
                let path = String::from_utf8_lossy(&output.stdout);
                println!("cargo:warning=Found protoc at: {}", path.trim());
            }
            _ => {
                println!("cargo:warning=protoc not found in PATH!");
            }
        }
        println!("cargo:warning=Compiling proto from: {:?}", proto);
        println!("cargo:warning=Output dir: {:?}", out_dir);
    }

    tonic_build::configure()
        .build_client(true)
        .build_server(true)
        .out_dir(out_dir)
        .compile_protos(&[proto], &[dir.join("proto")])?;
    Ok(())
}
