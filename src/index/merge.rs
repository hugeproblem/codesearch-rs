use std::io;
use std::cmp::Ordering;
use crate::index::read::{Index, DeltaReader};
use crate::index::write::{IndexBuffer, PathWriter, PathWriterState, PostDataWriter, IndexPath};

// Helper to check if name is covered by any root
fn is_shadowed(name: &str, roots: &[String]) -> bool {
    for root in roots {
        if name.starts_with(root) {
            return true;
        }
    }
    false
}

pub fn merge(dst_path: &str, src1_path: &str, src2_path: &str) -> io::Result<()> {
    let ix1 = Index::open(src1_path)?;
    let ix2 = Index::open(src2_path)?;

    // 1. Load roots from ix2 to determine shadowing
    let mut ix2_roots = Vec::new();
    let mut r = ix2.roots();
    while let Some(root) = r.next() {
        ix2_roots.push(root);
    }

    // 2. Build ID Map for ix1 (Old -> New)
    // -1 indicates shadowed/deleted.
    let mut id_map = vec![-1; ix1.num_name];
    let mut ix2_map = Vec::with_capacity(ix2.num_name);
    
    // Prepare Output Buffers
    let mut main_buf = IndexBuffer::new(dst_path)?;
    let mut name_buf = IndexBuffer::new("")?;
    let mut name_index_buf = IndexBuffer::new("")?;
    let mut post_buf = IndexBuffer::new("")?;
    let mut post_index_buf = IndexBuffer::new("")?;
    
    main_buf.write_string("csearch index 2\n")?;
    
    // 3. Write Merged Roots
    // Merge ix1.roots and ix2.roots
    let roots_off = main_buf.offset();
    let mut roots: Vec<String> = Vec::new();
    {
        let mut r1 = ix1.roots();
        while let Some(p) = r1.next() { roots.push(p); }
        let mut r2 = ix2.roots();
        while let Some(p) = r2.next() { roots.push(p); }
    }
    roots.sort(); 
    roots.dedup(); // Remove duplicates
    
    {
        let mut root_state = PathWriterState::new(16);
        let mut pw = PathWriter::new(&mut main_buf, None, &mut root_state);
        for r in &roots {
            pw.write(&IndexPath::new(r.clone()))?;
        }
    }
    let roots_count = roots.len(); // Go implementation counts paths
    main_buf.align(16)?;
    
    // 4. Merge Names
    let name_off = main_buf.offset();
    let mut name_count = 0;
    
    {
        let mut name_state = PathWriterState::new(16);
        let mut pw = PathWriter::new(&mut name_buf, Some(&mut name_index_buf), &mut name_state);
        
        let mut r1 = ix1.names_at(0, ix1.num_name);
        let mut r2 = ix2.names_at(0, ix2.num_name);
        
        let mut n1 = r1.next();
        let mut n2 = r2.next();
        
        let mut i1 = 0;
        let mut _i2 = 0; // tracking for debugging if needed
        
        while n1.is_some() || n2.is_some() {
            let mut take_1 = false;
            
            if n1.is_none() {
                // take_2
            } else if n2.is_none() {
                take_1 = true;
            } else {
                let s1 = n1.as_ref().unwrap();
                let s2 = n2.as_ref().unwrap();
                match s1.cmp(s2) {
                    Ordering::Less => take_1 = true,
                    Ordering::Greater => {}, // take_2
                    Ordering::Equal => {
                        // take_2 (shadows s1)
                    }
                }
            }
            
            if take_1 {
                let s = n1.unwrap();
                // Check shadowing
                if !is_shadowed(&s, &ix2_roots) {
                    pw.write(&IndexPath::new(s.clone()))?;
                    id_map[i1] = name_count as i32;
                    name_count += 1;
                }
                // else id_map[i1] = -1
                
                i1 += 1;
                n1 = r1.next();
            } else {
                // take_2
                let s = n2.unwrap();
                pw.write(&IndexPath::new(s.clone()))?;
                ix2_map.push(name_count as i32);
                
                // If s1 was equal, we need to skip it
                if n1.is_some() && n1.as_ref().unwrap() == &s {
                     // s1 is shadowed
                     i1 += 1;
                     n1 = r1.next();
                }
                
                _i2 += 1;
                n2 = r2.next();
                name_count += 1;
            }
        }
    }
    
    // 5. Merge Postings
    
    // 5. Merge Postings
    let mut trigram_count = 0;
    
    {
        let mut p1 = ix1.post_map_iter();
        let mut p2 = ix2.post_map_iter();
        
        let mut next1 = p1.next();
        let mut next2 = p2.next();
        
        let mut w = PostDataWriter::new(&mut post_buf, Some(&mut post_index_buf));
        
        while next1.is_some() || next2.is_some() {
            let mut t = u32::MAX;
            if let Some((t1, _, _)) = next1 { t = std::cmp::min(t, t1); }
            if let Some((t2, _, _)) = next2 { t = std::cmp::min(t, t2); }
            
            w.trigram(t)?;
            trigram_count += 1;
            
            let mut ids = Vec::new();
            
            if let Some((t1, count, offset)) = next1 {
                if t1 == t {
                    if ix1.post_data + offset + 3 <= ix1.mmap.len() {
                        let data = &ix1.mmap[ix1.post_data + offset + 3 ..];
                        let mut delta = DeltaReader::new(data);
                        let mut fileid = -1;
                        for _ in 0..count {
                            if let Some(d) = delta.next() {
                                fileid += d as i32;
                                if fileid >= 0 && (fileid as usize) < id_map.len() {
                                    let new_id = id_map[fileid as usize];
                                    if new_id != -1 {
                                        ids.push(new_id);
                                    }
                                }
                            }
                        }
                    }
                    next1 = p1.next();
                }
            }
            
            if let Some((t2, count, offset)) = next2 {
                if t2 == t {
                    if ix2.post_data + offset + 3 <= ix2.mmap.len() {
                        let data = &ix2.mmap[ix2.post_data + offset + 3 ..];
                        let mut delta = DeltaReader::new(data);
                        let mut fileid = -1;
                        for _ in 0..count {
                            if let Some(d) = delta.next() {
                                fileid += d as i32;
                                if fileid >= 0 && (fileid as usize) < ix2_map.len() {
                                    let new_id = ix2_map[fileid as usize];
                                    ids.push(new_id);
                                }
                            }
                        }
                    }
                    next2 = p2.next();
                }
            }
            
            ids.sort();
            ids.dedup();
            
            for id in ids {
                w.fileid(id)?;
            }
            w.end_trigram()?;
        }
        w.flush()?;
        }
        
        // 6. Write Trailer
    // We can reuse IndexWriter::flush logic partially? 
    // Or just write it manually since we have the buffers.
    
    main_buf.align(16)?;
    
    let mut name_f = name_buf.finish()?;
    let n = io::copy(&mut name_f, &mut main_buf.writer)?;
    main_buf.offset += n;
    main_buf.align(16)?;
    
    let post_off = main_buf.offset();
    let mut post_f = post_buf.finish()?;
    let n = io::copy(&mut post_f, &mut main_buf.writer)?;
    main_buf.offset += n;
    main_buf.align(16)?;
    
    let name_idx_off = main_buf.offset();
    let mut name_idx_f = name_index_buf.finish()?;
    let n = io::copy(&mut name_idx_f, &mut main_buf.writer)?;
    main_buf.offset += n;
    main_buf.align(16)?;
    
    let post_idx_off = main_buf.offset();
    let mut post_idx_f = post_index_buf.finish()?;
    let n = io::copy(&mut post_idx_f, &mut main_buf.writer)?;
    main_buf.offset += n;
    
    main_buf.write_uint64(roots_off)?;
    main_buf.write_uint64(roots_count as u64)?;
    main_buf.write_uint64(name_off)?;
    main_buf.write_uint64(name_count as u64)?;
    main_buf.write_uint64(post_off)?;
    main_buf.write_uint64(trigram_count as u64)?;
    main_buf.write_uint64(name_idx_off)?;
    main_buf.write_uint64(post_idx_off)?;
    main_buf.write_string("\ncsearch trlr 2\n")?;
    
    main_buf.flush()?;
    
    Ok(())
}
