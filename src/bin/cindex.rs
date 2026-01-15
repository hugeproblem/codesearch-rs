use clap::Parser;
use rust_codesearch::index::IndexWriter;
use rust_codesearch::find_index_file;
use ignore::WalkBuilder;
use std::collections::HashSet;
use std::path::Path;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(short, long, default_value = "")]
    index: String,

    #[arg(short, long)]
    verbose: bool,

    #[arg(short = 'n', long, help = "Do not respect .gitignore files")]
    no_ignore: bool,

    #[arg(short = 'a', long, help = "Index all file types (disable extension filtering)")]
    all_files: bool,

    #[arg(short = 'e', long, help = "Additional file extensions to index (comma-separated, e.g., 'rs,go,js')")]
    extensions: Option<String>,

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

fn main() -> anyhow::Result<()> {
    env_logger::init();
    let args = Args::parse();

    // Build allowed extensions set
    let mut allowed_extensions = get_default_extensions();
    
    // Add user-specified extensions
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
        args.index
    };

    let mut ix = IndexWriter::create(&index_file)?;
    println!("Creating index at: {}", index_file);
    ix.verbose = args.verbose;

    for path in args.paths {
        let mut builder = WalkBuilder::new(&path);
        
        // Configure gitignore handling
        if args.no_ignore {
            builder.ignore(false);
            builder.git_ignore(false);
            builder.git_global(false);
            builder.git_exclude(false);
        }
        
        for entry in builder.build() {
            let entry = entry?;
            if entry.file_type().map_or(false, |ft| ft.is_file()) {
                let path = entry.path();
                
                // Check if file should be indexed based on extension
                if should_index_file(path, &allowed_extensions, args.all_files) {
                    let path_str = path.to_string_lossy();
                    if args.verbose {
                        println!("{}", path_str);
                    }
                    ix.add_file(&path_str)?;
                } else if args.verbose {
                    println!("Skipping: {}", path.to_string_lossy());
                }
            }
        }
    }

    ix.flush()?;
    Ok(())
}
