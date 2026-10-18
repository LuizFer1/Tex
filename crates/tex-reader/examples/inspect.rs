use std::fs::File;
use std::path::PathBuf;
use std::process::exit;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use tex_reader::{Block, Body, Kind, LoadError, blocks, probe};

const SHOWN_CHARS: usize = 200;

fn main() {
    let mut args = std::env::args_os().skip(1);
    let Some(path) = args.next().map(PathBuf::from) else {
        eprintln!("uso: inspect <arquivo> [quantos itens mostrar, padrão 30]");
        exit(2);
    };
    let count: usize = args
        .next()
        .and_then(|value| value.to_str()?.parse().ok())
        .unwrap_or(30);

    let probed = match probe(&path, &[]) {
        Ok(probed) => probed,
        Err(error) => fail(error),
    };
    println!("tipo:            {:?}", probed.kind);
    println!("tamanho:         {} bytes", probed.stamp.len);
    if probed.large() {
        println!(
            "prévia:          {} linhas nos primeiros 64 KB",
            probed.preview_lines().len()
        );
    }

    let started = Instant::now();
    let document = match probed.load(&AtomicBool::new(false)) {
        Ok(Some(document)) => document,
        Ok(None) => unreachable!("load is never cancelled here"),
        Err(error) => fail(error),
    };
    println!("carregado em:    {:?}", started.elapsed());
    println!("codificação:     {:?}", document.encoding);
    println!("fim de linha:    {:?}", document.line_ending);
    println!("editável:        {}", document.editable());
    println!("salva sem perda: {}", document.fits());
    println!();

    match &document.body {
        Body::Text(text) if document.kind == Kind::Markdown => {
            let blocks = blocks(text);
            println!(
                "{} blocos de Markdown (mostrando até {count}):",
                blocks.len()
            );
            println!();
            for block in blocks.iter().take(count) {
                print_block(block);
            }
        }
        Body::Text(text) => {
            let lines: Vec<&str> = text.split('\n').collect();
            println!("{} linhas em memória (mostrando até {count}):", lines.len());
            println!();
            for (number, line) in lines.iter().take(count).enumerate() {
                println!("{:>6}  {}", number + 1, shorten(line));
            }
        }
        Body::Lines(index) => {
            println!(
                "{} linhas indexadas, só leitura (mostrando até {count}):",
                index.len()
            );
            println!();
            let mut file = File::open(&document.path).unwrap_or_else(|error| fail(error.into()));
            let lines = index
                .read_lines(&mut file, 0..count)
                .unwrap_or_else(|error| fail(error.into()));
            for (number, line) in lines.iter().enumerate() {
                println!("{:>6}  {}", number + 1, shorten(line));
            }
        }
    }
}

fn print_block(block: &Block) {
    match block {
        Block::Heading { level, inline } => {
            println!("{} {}", "#".repeat(usize::from(*level)), shorten(inline))
        }
        Block::Paragraph { inline } => println!("¶ {}", shorten(inline)),
        Block::Item {
            depth,
            marker,
            inline,
        } => {
            let indent = "  ".repeat(depth.saturating_sub(1));
            let marker = if marker.is_empty() { " " } else { marker };
            println!("{indent}{marker} {}", shorten(inline));
        }
        Block::Code { lang, text } => {
            println!("```{lang}");
            for line in text.lines() {
                println!("{}", shorten(line));
            }
            println!("```");
        }
        Block::Quote { depth, inline } => println!("{} {}", ">".repeat(*depth), shorten(inline)),
        Block::Rule => println!("---"),
        Block::Table { header, rows } => {
            println!("| {} |", header.join(" | "));
            for row in rows {
                println!("| {} |", row.join(" | "));
            }
        }
        Block::Image { alt } => println!("[imagem: {alt}]"),
    }
    println!();
}

fn shorten(text: &str) -> String {
    match text.char_indices().nth(SHOWN_CHARS) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}

fn fail(error: LoadError) -> ! {
    match error {
        LoadError::Unsupported => eprintln!("o leitor não abre este tipo de arquivo"),
        LoadError::Binary => eprintln!("o arquivo não tem extensão e não é texto"),
        LoadError::Io(error) => eprintln!("erro ao ler: {error}"),
    }
    exit(1);
}
