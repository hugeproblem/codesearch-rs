use clap::Parser;
use rust_codesearch::index::IndexWriter;
use rust_codesearch::index::merge::merge;
use rust_codesearch::index::read::Index;
use rust_codesearch::find_index_file;
use ignore::WalkBuilder;
use std::collections::HashSet;
use std::path::Path;
use std::fs;
use std::io::{BufRead, BufWriter, Write};
use std::time::Instant;

/// Checkpoint file stores progress for resumable indexing
const CHECKPOINT_INTERVAL: usize = 10000; // Save checkpoint every N files

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(short, long, default_value = "")]
    index: String,

    #[arg(short, long)]
    verbose: bool,

    #[arg(short = 'n', long, help = "Do not respect .gitignore files")]
    no_ignore: bool,
    
    #[arg(long, help = "Overwrite existing index")]
    reset: bool,

    #[arg(short = 'a', long, help = "Index all file types (disable extension filtering)")]
    all_files: bool,

    #[arg(short = 'e', long, help = "Additional file extensions to index (comma-separated, e.g., 'rs,go,js')")]
    extensions: Option<String>,

    #[arg(long, help = "Checkpoint interval (save progress every N files) [default: 10000]")]
    checkpoint_interval: Option<usize>,

    #[arg(long, help = "Resume from checkpoint if available")]
    resume: bool,

    #[arg(required = true)]
    paths: Vec<String>,
}

fn get_default_extensions() -> HashSet<String> {
    let extensions = [
        // Text files
        "txt", "md", "rst", "org",
        // C/C++
        "c", "h", "cpp", "hpp", "cc", "hh", "cxx", "hxx", "inl",
        // Python
        "py", "pyw", "pyi",
        // Rust
        "rs",
        // Go
        "go",
        // JavaScript/TypeScript
        "js", "jsx", "ts", "tsx", "mjs",
        // Java
        "java",
        // C#
        "cs",
        // Shell scripts
        "sh", "bash", "zsh", "fish",
        // Web
        "html", "htm", "css", "scss", "sass", "less",
        // Config files
        "json", "yaml", "yml", "toml", "ini", "cfg", "conf",
        // Other common text formats
        "xml", "svg", "sql", "cmake", "make", "dockerfile",
        // Assembly
        "s", "asm",
        // Perl
        "pl", "pm",
        // Ruby
        "rb",
        // PHP
        "php",
        // Lua
        "lua",
        // Swift
        "swift",
        // Kotlin
        "kt", "kts",
        // Scala
        "scala",
        // Clojure
        "clj", "cljs",
        // Haskell
        "hs",
        // OCaml
        "ml", "mli",
        // Erlang
        "erl", "hrl",
        // Elixir
        "ex", "exs",
        // R
        "r",
        // MATLAB
        "m",
        // Vim
        "vim",
        // LaTeX
        "tex", "sty",
    ];
    
    extensions.iter().map(|&s| s.to_string()).collect()
}

fn should_index_file(path: &Path, allowed_extensions: &HashSet<String>, index_all: bool) -> bool {
    if index_all {
        return true;
    }
    
    if let Some(extension) = path.extension() {
        if let Some(ext_str) = extension.to_str() {
            return allowed_extensions.contains(&ext_str.to_lowercase());
        }
    }
    
    // Also index files without extensions (like Makefile, Dockerfile, etc.)
    if path.extension().is_none() {
        if let Some(filename) = path.file_name() {
            if let Some(name_str) = filename.to_str() {
                let name_lower = name_str.to_lowercase();
                return matches!(name_lower.as_str(), 
                    "makefile" | "dockerfile" | "cmakelists.txt" | "readme" | 
                    "license" | "authors" | "contributors" | "changelog" | 
                    "news" | "todo" | "install" | "copying" | "notice"
                );
            }
        }
    }
    
    false
}

/// Get checkpoint file path for an index file
fn get_checkpoint_path(index_file: &str) -> String {
    format!("{}.checkpoint", index_file)
}

/// Get checkpoint index file path (partial index)
fn get_checkpoint_index_path(index_file: &str) -> String {
    format!("{}.checkpoint.idx", index_file)
}

/// Load checkpoint - returns set of already indexed files
fn load_checkpoint(checkpoint_path: &str) -> HashSet<String> {
    let mut indexed = HashSet::new();
    if let Ok(file) = fs::File::open(checkpoint_path) {
        let reader = std::io::BufReader::new(file);
        for line in reader.lines() {
            if let Ok(path) = line {
                if !path.is_empty() {
                    indexed.insert(path);
                }
            }
        }
    }
    indexed
}

/// Save checkpoint - write all indexed files to checkpoint file
fn save_checkpoint(checkpoint_path: &str, indexed_files: &[String]) -> anyhow::Result<()> {
    let file = fs::File::create(checkpoint_path)?;
    let mut writer = BufWriter::new(file);
    for path in indexed_files {
        writeln!(writer, "{}", path)?;
    }
    writer.flush()?;
    Ok(())
}

/// Remove checkpoint files after successful completion
fn cleanup_checkpoint(index_file: &str) {
    let checkpoint_path = get_checkpoint_path(index_file);
    let checkpoint_idx_path = get_checkpoint_index_path(index_file);
    let _ = fs::remove_file(&checkpoint_path);
    let _ = fs::remove_file(&checkpoint_idx_path);
}

/// Checkpoint state for resumable indexing
struct CheckpointState {
    indexed_files: Vec<String>,
    checkpoint_path: String,
    interval: usize,
    last_checkpoint: usize,
    verbose: bool,
}

impl CheckpointState {
    fn new(index_file: &str, interval: usize, verbose: bool) -> Self {
        CheckpointState {
            indexed_files: Vec::new(),
            checkpoint_path: get_checkpoint_path(index_file),
            interval,
            last_checkpoint: 0,
            verbose,
        }
    }
    
    fn add_file(&mut self, path: String) {
        self.indexed_files.push(path);
    }
    
    fn should_checkpoint(&self) -> bool {
        self.indexed_files.len() - self.last_checkpoint >= self.interval
    }
    
    fn save(&mut self, _ix: &mut IndexWriter) -> anyhow::Result<()> {
        // Save list of indexed files
        save_checkpoint(&self.checkpoint_path, &self.indexed_files)?;
        
        // Note: We only save the file list, not the partial index state.
        // On resume, files in the checkpoint will be skipped and only new files indexed.
        // This is simpler and safer than trying to serialize partial index state.
        
        self.last_checkpoint = self.indexed_files.len();
        
        if self.verbose {
            eprintln!("Checkpoint saved: {} files indexed", self.indexed_files.len());
        }
        
        Ok(())
    }
}

fn index_paths(ix: &mut IndexWriter, paths: &[String], args: &Args, allowed_extensions: &HashSet<String>, index_file: &str) -> anyhow::Result<()> {
    let checkpoint_interval = args.checkpoint_interval.unwrap_or(CHECKPOINT_INTERVAL);
    let checkpoint_path = get_checkpoint_path(index_file);
    
    // Load existing checkpoint if resuming
    let already_indexed: HashSet<String> = if args.resume {
        let indexed = load_checkpoint(&checkpoint_path);
        if !indexed.is_empty() && args.verbose {
            eprintln!("Resuming from checkpoint: {} files already indexed", indexed.len());
        }
        indexed
    } else {
        HashSet::new()
    };
    
    let mut checkpoint_state = CheckpointState::new(index_file, checkpoint_interval, args.verbose);
    let start_time = Instant::now();
    let mut files_processed = 0;
    let mut files_skipped = 0;
    
    for path in paths {
        let abs_path = if let Ok(p) = fs::canonicalize(path) {
             p.to_string_lossy().to_string()
        } else {
             path.clone()
        };
        
        ix.add_root(&abs_path);

        let mut builder = WalkBuilder::new(path);
        
        if args.no_ignore {
            builder.ignore(false);
            builder.git_ignore(false);
            builder.git_global(false);
            builder.git_exclude(false);
        }
        
        let mut files = Vec::new();
        
        for entry in builder.build() {
            let entry = entry?;
            if entry.file_type().map_or(false, |ft| ft.is_file()) {
                let path = entry.path();
                
                if should_index_file(path, allowed_extensions, args.all_files) {
                     let path_str = if let Ok(p) = fs::canonicalize(path) {
                         p.to_string_lossy().to_string()
                     } else {
                         path.to_string_lossy().to_string()
                     };
                     files.push(path_str);
                } else if args.verbose {
                    println!("Skipping: {}", path.to_string_lossy());
                }
            }
        }
        
        files.sort();
        
        let total_files = files.len();
        
        for path_str in files {
            // Skip if already indexed (from checkpoint)
            if already_indexed.contains(&path_str) {
                files_skipped += 1;
                continue;
            }
            
            if args.verbose {
                println!("{}", path_str);
            }
            ix.add_file(&path_str)?;
            
            checkpoint_state.add_file(path_str);
            files_processed += 1;
            
            // Save checkpoint periodically
            if checkpoint_state.should_checkpoint() {
                checkpoint_state.save(ix)?;
                
                if args.verbose {
                    let elapsed = start_time.elapsed().as_secs_f64();
                    let rate = files_processed as f64 / elapsed;
                    eprintln!("Progress: {}/{} files ({:.1} files/sec)", 
                             files_processed + files_skipped, total_files, rate);
                }
            }
        }
    }
    
    ix.flush()?;
    
    // Cleanup checkpoint on successful completion
    cleanup_checkpoint(index_file);
    
    if args.verbose {
        let elapsed = start_time.elapsed().as_secs_f64();
        eprintln!("Indexing complete: {} files indexed, {} skipped (resumed), {:.1}s", 
                 files_processed, files_skipped, elapsed);
    }
    
    Ok(())
}

fn main() -> anyhow::Result<()> {
    env_logger::init();
    let args = Args::parse();

    let mut allowed_extensions = get_default_extensions();
    if let Some(ref ext_list) = args.extensions {
        for ext in ext_list.split(',') {
            let ext = ext.trim().to_lowercase();
            if !ext.is_empty() {
                allowed_extensions.insert(ext);
            }
        }
    }
    
    if args.verbose && !args.all_files {
        println!("Indexing files with extensions: {:?}", 
                 allowed_extensions.iter().collect::<Vec<_>>());
    }

    let index_file = if args.index.is_empty() {
        find_index_file(true)?
    } else {
        args.index.clone()
    };
    
    let path_exists = Path::new(&index_file).exists();
    
    // Check if existing index is valid
    let index_valid = if path_exists {
        Index::open(&index_file).is_ok()
    } else {
        false
    };
    
    // Check for existing checkpoint
    let checkpoint_path = get_checkpoint_path(&index_file);
    let has_checkpoint = Path::new(&checkpoint_path).exists();
    
    // Decide whether to create new index or update existing
    // - Create new if: reset flag, no index exists, index is invalid, or resuming with checkpoint
    let should_create_new = args.reset || !path_exists || !index_valid || (args.resume && has_checkpoint);
    
    if should_create_new {
        if args.resume && has_checkpoint && args.verbose {
            println!("Found checkpoint, will resume indexing");
        }
        
        if args.verbose {
            if !index_valid && path_exists && !has_checkpoint {
                println!("Existing index is invalid, overwriting: {}", index_file);
            } else if !args.resume || !has_checkpoint {
                println!("Creating index at: {}", index_file);
            }
        }
        let mut ix = IndexWriter::create(&index_file)?;
        ix.verbose = args.verbose;
        ix.log_skip = args.verbose;
        index_paths(&mut ix, &args.paths, &args, &allowed_extensions, &index_file)?;
    } else {
        if args.verbose { println!("Updating index at: {}", index_file); }
        
        let temp_new = format!("{}.tmp_new", index_file);
        let temp_merged = format!("{}.tmp_merged", index_file);
        
        let mut ix = IndexWriter::create(&temp_new)?;
        ix.verbose = args.verbose;
        ix.log_skip = args.verbose;
        index_paths(&mut ix, &args.paths, &args, &allowed_extensions, &index_file)?;
        
        // Merge
        match merge(&temp_merged, &index_file, &temp_new) {
            Ok(_) => {
                fs::rename(&temp_merged, &index_file)?;
                let _ = fs::remove_file(&temp_new);
                // Cleanup checkpoint for the main index file
                cleanup_checkpoint(&index_file);
            }
            Err(e) => {
                let _ = fs::remove_file(&temp_new);
                let _ = fs::remove_file(&temp_merged);
                return Err(e.into());
            }
        }
    }

    Ok(())
}