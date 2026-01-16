use rust_codesearch::index::read::Index;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    index: String,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let ix = Index::open(&args.index)?;
    
    println!("Roots ({}):", ix.num_path);
    let mut r = ix.roots();
    while let Some(p) = r.next() {
        println!("  {}", p);
    }
    
    println!("Name Data Offset: {}", ix.name_data);
    if ix.name_data < ix.mmap.len() {
        let len = std::cmp::min(50, ix.mmap.len() - ix.name_data);
        let slice = &ix.mmap[ix.name_data .. ix.name_data + len];
        println!("Name Data Header: {:?}", slice);
    }

    println!("Names ({}):", ix.num_name);
    let mut n = ix.names_at(0, ix.num_name);
    while let Some(p) = n.next() {
        println!("  {}", p);
    }
    
    println!("Postings ({}):", ix.num_post);
    let mut p = ix.post_map_iter();
    while let Some((t, count, offset)) = p.next() {
        let c = (t as u8) as char;
        let b = ((t >> 8) as u8) as char;
        let a = ((t >> 16) as u8) as char;
        let display_a = if a.is_ascii_graphic() { a } else { '.' };
        let display_b = if b.is_ascii_graphic() { b } else { '.' };
        let display_c = if c.is_ascii_graphic() { c } else { '.' };
        
        println!("  Trigram '{}{}{}' ({}): count={} offset={}", display_a, display_b, display_c, t, count, offset);
    }
    
    Ok(())
}
