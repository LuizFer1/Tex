#[allow(dead_code)]
#[path = "../src/update.rs"]
mod update;

use std::env;
use std::fs;
use std::process::ExitCode;

use ed25519_dalek::{Signer, SigningKey};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let [installer, version] = args.as_slice() else {
        eprintln!(
            "usage: sign_update <installer> <version>  (TEX_UPDATE_KEY holds the private key)"
        );
        return ExitCode::FAILURE;
    };
    let Some(version) = update::Version::parse(version) else {
        eprintln!("not a version: {version}");
        return ExitCode::FAILURE;
    };
    let Some(seed) = env::var("TEX_UPDATE_KEY")
        .ok()
        .and_then(|key| update::from_hex(&key))
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
    else {
        eprintln!("TEX_UPDATE_KEY must hold the 32-byte private key in hex");
        return ExitCode::FAILURE;
    };
    let key = SigningKey::from_bytes(&seed);
    if key.verifying_key().to_bytes() != update::PUBLIC_KEY {
        eprintln!("TEX_UPDATE_KEY does not match crates/tex-app/update.pub");
        return ExitCode::FAILURE;
    }
    let bytes = match fs::read(installer) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("could not read {installer}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let message = update::signed_message(version, &update::sha256_hex(&bytes));
    let signature = key.sign(&message).to_bytes();
    if !update::verify(&bytes, version, &signature, &update::PUBLIC_KEY) {
        eprintln!("the signature does not verify");
        return ExitCode::FAILURE;
    }
    let path = format!("{installer}.sig");
    if let Err(error) = fs::write(&path, format!("{}\n", update::to_hex(&signature))) {
        eprintln!("could not write {path}: {error}");
        return ExitCode::FAILURE;
    }
    println!("{path}");
    ExitCode::SUCCESS
}
