use clap::Parser;
use flatten_repo::{Config, FlattenRepo};
use std::{
    io::{self, BufRead},
    process,
};

/// Combines multiple files into a single XML document while respecting gitignore rules.
#[derive(Parser)]
#[command(name = "flatten-repo")]
struct Cli {
    /// Recursively process directories
    #[arg(short = 'R', short_alias = 'r', long, default_value = "true")]
    recursive: bool,

    /// Enable verbose output
    #[arg(short = 'V', short_alias = 'v', long)]
    verbose: bool,

    /// Ignore files matching pattern (can be specified multiple times)
    #[arg(short = 'I', short_alias = 'i', long)]
    ignore: Vec<String>,

    /// File paths or globs to process. Additional paths can be provided via STDIN.
    paths: Vec<String>,
}

// Read paths from STDIN, one per line, skipping empty lines
fn read_paths_from_stdin() -> io::Result<Vec<String>> {
    let stdin = io::stdin();
    let mut paths = Vec::new();

    if atty::is(atty::Stream::Stdin) {
        return Ok(paths);
    }

    for line in stdin.lock().lines() {
        let path = line?;
        if !path.trim().is_empty() {
            paths.push(path);
        }
    }

    Ok(paths)
}

fn main() {
    let args = Cli::parse();

    // Combine paths from arguments and STDIN
    let mut paths = args.paths;
    match read_paths_from_stdin() {
        Ok(mut stdin_paths) => paths.append(&mut stdin_paths),
        Err(e) => {
            eprintln!("Error reading from STDIN: {}", e);
            process::exit(1);
        }
    }

    // Only use current directory if no paths from either source
    if paths.is_empty() {
        paths.push(".".to_string());
    }

    let config = Config {
        recursive: args.recursive,
        verbose: args.verbose,
        ignore_patterns: args.ignore,
        paths,
    };

    match FlattenRepo::new(config) {
        Ok(flattener) => match flattener.generate_xml() {
            Ok(xml) => println!("{}", xml),
            Err(e) => {
                eprintln!("Error generating XML: {}", e);
                process::exit(1);
            }
        },
        Err(e) => {
            eprintln!("Error initializing: {}", e);
            process::exit(1);
        }
    }
}