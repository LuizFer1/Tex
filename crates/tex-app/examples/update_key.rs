use std::env;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

use ed25519_dalek::SigningKey;

fn main() -> ExitCode {
    let Some(secret) = env::args().nth(1) else {
        eprintln!("usage: update_key <file for the private key, outside the repository>");
        return ExitCode::FAILURE;
    };
    let secret = Path::new(&secret);
    let public = Path::new(env!("CARGO_MANIFEST_DIR")).join("update.pub");
    if public.exists() {
        eprintln!(
            "{} already exists. Replacing it stops every installed Tex from accepting updates.",
            public.display()
        );
        return ExitCode::FAILURE;
    }
    if secret.exists() {
        eprintln!("{} already exists", secret.display());
        return ExitCode::FAILURE;
    }
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).expect("system random generator");
    let key = SigningKey::from_bytes(&seed);
    let hex: String = seed.iter().map(|byte| format!("{byte:02x}")).collect();
    if let Err(error) = fs::write(secret, format!("{hex}\n")) {
        eprintln!("could not write {}: {error}", secret.display());
        return ExitCode::FAILURE;
    }
    if let Err(error) = fs::write(&public, key.verifying_key().to_bytes()) {
        eprintln!("could not write {}: {error}", public.display());
        return ExitCode::FAILURE;
    }
    println!("Public key: {}", public.display());
    println!("Private key: {}", secret.display());
    println!("Set it as the TEX_UPDATE_KEY secret and keep a backup of the file.");
    ExitCode::SUCCESS
}
