use clap::Parser;
use flatten_repo::{Config, FlattenRepo};
use std::process;

/// Combines multiple files into a single XML document while respecting gitignore rules.
#[derive(Parser)]
#[command(name = "flatten-repo")]
struct Cli {
    /// Recursively process directories
    #[arg(short = 'R', short_alias = 'r', long)]
    recursive: bool,

    /// Enable verbose output
    #[arg(short = 'V', short_alias = 'v', long)]
    verbose: bool,

    /// Ignore files matching pattern (can be specified multiple times)
    #[arg(short = 'I', short_alias = 'i', long)]
    ignore: Vec<String>,

    /// File paths or globs to process
    paths: Vec<String>,
}

fn main() {
    let args = Cli::parse();

    let config = Config {
        recursive: args.recursive,
        verbose: args.verbose,
        ignore_patterns: args.ignore,
        paths: args.paths,
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