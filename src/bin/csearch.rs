use anyhow::{Context, Result};
use clap::Parser;
use rust_codesearch::index::{Index, regexp};
use std::fs::File;
use std::io::{BufRead, BufReader};
use regex::RegexBuilder;
use std::path::Path;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// The index file to use
    #[arg(short = 'x', long)]
    index: Option<String>,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,

    /// Case-insensitive search
    #[arg(short = 'i', long)]
    ignore_case: bool,
    
    /// Print line number
    #[arg(short = 'n', long)]
    line_number: bool,

    /// The pattern to search for
    pattern: String,
}

fn main() -> Result<()> {
    let args = Args::parse();
    
    // Open index
    let index_path = if let Some(p) = args.index {
        p
    } else {
        std::env::var("CSEARCHINDEX").unwrap_or_else(|_| {
             let home = std::env::var("HOME").expect("HOME not set");
             format!("{}/.csearchindex", home)
        })
    };
    
    let index = Index::open(&index_path).context(format!("failed to open index {}", index_path))?;
    
    let pattern = if args.ignore_case {
        // Check if pattern already has (?i) to avoid double prefix if user provided it?
        // But prepending is safe usually.
        format!("(?i){}", args.pattern)
    } else {
        args.pattern.clone()
    };
    
    if args.verbose {
        eprintln!("pattern: {}", pattern);
    }

    let q = regexp::analyze_regexp(&pattern).context("failed to analyze regexp")?;
    
    if args.verbose {
        eprintln!("query: {:?}", q);
    }
    
    let post = index.posting_query(&q);
    
    if args.verbose {
        eprintln!("post query identified {} possible files", post.len());
    }
    
    let re = RegexBuilder::new(&pattern)
        .case_insensitive(args.ignore_case)
        .build()
        .context("failed to compile regex")?;
        
    for fileid in post {
        let name = index.name(fileid as usize);
        let path = Path::new(&name);
        
        let file = match File::open(path) {
            Ok(f) => f,
            Err(_) => continue,
        };
        
        let reader = BufReader::new(file);
        
        for (i, line_res) in reader.lines().enumerate() {
            if let Ok(line) = line_res {
                if re.is_match(&line) {
                    if args.line_number {
                        println!("{}:{}:{}", name, i+1, line);
                    } else {
                        println!("{}:{}", name, line);
                    }
                }
            }
        }
    }
    
    Ok(())
}
