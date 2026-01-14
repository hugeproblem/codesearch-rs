use clap::Parser;
use rust_codesearch::index::IndexWriter;
use walkdir::WalkDir;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(short, long, default_value = "")]
    index: String,

    #[arg(short, long)]
    verbose: bool,

    #[arg(required = true)]
    paths: Vec<String>,
}

fn main() -> anyhow::Result<()> {
    env_logger::init();
    let args = Args::parse();

    let index_file = if args.index.is_empty() {
        std::env::var("CSEARCHINDEX").unwrap_or_else(|_| {
            let home = std::env::var("HOME").expect("HOME not set");
            format!("{}/.csearchindex", home)
        })
    } else {
        args.index
    };

    let mut ix = IndexWriter::create(&index_file)?;
    println!("Creating index at: {}", index_file);
    ix.verbose = args.verbose;

    for path in args.paths {
        for entry in WalkDir::new(&path) {
            let entry = entry?;
            if entry.file_type().is_file() {
                let path_str = entry.path().to_string_lossy();
                if args.verbose {
                    println!("{}", path_str);
                }
                ix.add_file(&path_str)?;
            }
        }
    }

    ix.flush()?;
    Ok(())
}
