use anyhow::{Context, Result};
use clap::Parser;
use rust_codesearch::index::{Index, regexp};
use rust_codesearch::find_index_file;
use std::fs::File;
use std::io::{BufRead, BufReader};
use regex::bytes::RegexBuilder;
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
        find_index_file(false)?
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
        eprintln!("Index info: num_name={}, num_post={}, name_data={}, name_index={}, post_data={}, post_index={}", 
                  index.num_name, index.num_post, index.name_data, index.name_index, index.post_data, index.post_index);
    }

    let q = regexp::analyze_regexp(&pattern).context("failed to analyze regexp")?;
    
    if args.verbose {
        eprintln!("query: {:?}", q);
    }
    
    let post = index.posting_query(&q);
    
    if args.verbose {
        eprintln!("post query identified {} possible files", post.len());
        // Debug: check for duplicates
        let unique: std::collections::HashSet<_> = post.iter().collect();
        if unique.len() != post.len() {
            eprintln!("WARNING: posting_query returned {} duplicates!", post.len() - unique.len());
        }
    }
    
    let re = RegexBuilder::new(&pattern)
        .case_insensitive(args.ignore_case)
        .build()
        .context("failed to compile regex")?;
        
    for fileid in post {
        let name = index.name(fileid as usize);
        if name.is_empty() {
            if args.verbose {
                eprintln!("Warning: empty filename for fileid {}", fileid);
            }
            continue;
        }
        
        let path = Path::new(&name);
        
        let file = match File::open(path) {
            Ok(f) => f,
            Err(e) => {
                if args.verbose {
                    eprintln!("Warning: failed to open {}: {}", name, e);
                }
                continue;
            }
        };
        
        let reader = BufReader::new(file);
        
        let mut line_num = 0;
        for line_res in reader.split(b'\n') {
            line_num += 1;
            match line_res {
                Ok(line_bytes) => {
                    if re.is_match(&line_bytes) {
                        // Convert to string (lossy)
                        let line = String::from_utf8_lossy(&line_bytes);
                        // Remove trailing \r if present
                        let line = line.trim_end_matches('\r');
                        if args.line_number {
                            println!("{}:{}:{}", name, line_num, line);
                        } else {
                            println!("{}:{}", name, line);
                        }
                    }
                }
                Err(e) => {
                    if args.verbose {
                        eprintln!("Warning: error reading line {} from {}: {}", line_num, name, e);
                    }
                    break;
                }
            }
        }
    }
    
    Ok(())
}
