use memmap2::Mmap;
use std::fs::File;
use std::path::Path as StdPath;
use std::io;
use std::str;
use crate::index::regexp::{Query, QueryOp};
use byteorder::{BigEndian, ByteOrder};

// Helper function to read 24-bit big-endian integer
fn read_u24_be(buf: &[u8]) -> u32 {
    if buf.len() < 3 {
        return 0;
    }
    ((buf[0] as u32) << 16) | ((buf[1] as u32) << 8) | (buf[2] as u32)
}

// Constants
const TRAILER_MAGIC_V2: &str = "\ncsearch trlr 2\n";
const POST_BLOCK_SIZE: usize = 256;
const NAME_GROUP_SIZE: usize = 16;
const DELTA_ZERO_ENC: u32 = 16;

pub struct Index {
    pub mmap: Mmap,
    
    // Offsets/Counts
    pub path_data: usize,
    pub num_path: usize,
    pub name_data: usize,
    pub num_name: usize,
    pub post_data: usize,
    pub num_post: usize,
    pub name_index: usize,
    pub post_index: usize,
    pub num_post_block: usize,
}

impl Index {
    pub fn open<P: AsRef<StdPath>>(path: P) -> io::Result<Self> {
        let file = File::open(path)?;
        let mmap = unsafe { Mmap::map(&file)? };
        
        if mmap.len() < TRAILER_MAGIC_V2.len() {
             return Err(io::Error::new(io::ErrorKind::InvalidData, "file too short"));
        }
        
        let trailer_len = TRAILER_MAGIC_V2.len();
        let magic_start = mmap.len() - trailer_len;
        if &mmap[magic_start..] != TRAILER_MAGIC_V2.as_bytes() {
             return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid trailer magic"));
        }
        
        let n = magic_start as isize - 8 * 8;
        if n < 0 {
             return Err(io::Error::new(io::ErrorKind::InvalidData, "file too short for trailer"));
        }
        let n = n as usize;
        
        // Ensure we have enough data to read all the trailer fields
        if n + 64 > mmap.len() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid trailer size"));
        }
        
        let path_data = BigEndian::read_u64(&mmap[n..n+8]) as usize;
        let num_path = BigEndian::read_u64(&mmap[n+8..n+16]) as usize;
        let name_data = BigEndian::read_u64(&mmap[n+16..n+24]) as usize;
        let num_name = BigEndian::read_u64(&mmap[n+24..n+32]) as usize;
        let post_data = BigEndian::read_u64(&mmap[n+32..n+40]) as usize;
        let num_post = BigEndian::read_u64(&mmap[n+40..n+48]) as usize;
        let name_index = BigEndian::read_u64(&mmap[n+48..n+56]) as usize;
        let post_index = BigEndian::read_u64(&mmap[n+56..n+64]) as usize;
        
        // Validate offsets are within file bounds
        if path_data >= mmap.len() || name_data >= mmap.len() || 
           post_data >= mmap.len() || name_index >= mmap.len() || 
           post_index >= mmap.len() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid section offsets"));
        }
        
        // Validate ordering: path_data <= name_data <= post_data <= name_index <= post_index
        if path_data > name_data || name_data > post_data || 
           post_data > name_index || name_index > post_index {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid section ordering"));
        }
        
        let num_post_block = if post_index <= n {
            (n - post_index) / POST_BLOCK_SIZE
        } else {
            0
        };

        Ok(Index {
            mmap,
            path_data,
            num_path,
            name_data,
            num_name,
            post_data,
            num_post,
            name_index,
            post_index,
            num_post_block,
        })
    }
    
    fn slice_from(&self, off: usize) -> &[u8] {
        &self.mmap[off..]
    }
    
    fn uint64(&self, off: usize) -> u64 {
        if off + 8 > self.mmap.len() {
            return 0;
        }
        BigEndian::read_u64(&self.mmap[off..off+8])
    }
    
    pub fn name(&self, fileid: usize) -> String {
         let mut r = self.names_at(fileid, fileid + 1);
         r.next().unwrap_or_default()
    }
    
    pub fn names_at(&self, min: usize, max: usize) -> PathReader<'_> {
        if min >= self.num_name || max <= min {
            return PathReader::new(&[], 0);
        }
        let mut limit = max - min;
        let off_idx = self.name_index + (min / NAME_GROUP_SIZE) * 8;
        // Check bounds for name_index access
        if off_idx + 8 > self.mmap.len() {
            return PathReader::new(&[], 0);
        }
        
        let off = self.uint64(off_idx) as usize;
        
        let skip = min % NAME_GROUP_SIZE;
        limit += skip;
        
        let data_start = self.name_data + off;
        let end = self.post_data;
        
        // Check bounds
        if data_start >= end || data_start >= self.mmap.len() || end > self.mmap.len() {
            return PathReader::new(&[], 0);
        }
        
        let data = &self.mmap[data_start .. end];
        
        let mut r = PathReader::new(data, limit);
        for _ in 0..skip {
            if r.next().is_none() {
                break;
            }
        }
        r
    }
    
    pub fn roots(&self) -> PathReader<'_> {
        self.roots_at(0, self.num_path)
    }

    pub fn roots_at(&self, min: usize, max: usize) -> PathReader<'_> {
        if min >= self.num_path || max <= min {
            return PathReader::new(&[], 0);
        }
        let limit = max - min;
        // Roots are stored starting at path_data.
        
        let data_start = self.path_data;
        let end = self.name_data;
        
        if data_start >= end || data_start >= self.mmap.len() || end > self.mmap.len() {
             return PathReader::new(&[], 0);
        }
        
        let data = &self.mmap[data_start .. end];
        
        let mut r = PathReader::new(data, limit);
        for _ in 0..min {
            if r.next().is_none() {
                break;
            }
        }
        r
    }

    pub fn post_map_iter(&self) -> PostMapIter<'_> {
        PostMapIter::new(self)
    }
    
    pub fn posting_query(&self, q: &Query) -> Vec<u32> {
        self.posting_query_rec(q, None)
    }
    
    fn posting_query_rec(&self, q: &Query, restrict: Option<Vec<u32>>) -> Vec<u32> {
        match q.op {
            QueryOp::None => Vec::new(),
            QueryOp::All => {
                if let Some(r) = restrict {
                    return r;
                }
                (0..self.num_name as u32).collect()
            }
            QueryOp::And => {
                let mut list = None;
                for t in &q.trigram {
                    let tri = trigram_u32(t);
                    if list.is_none() {
                        list = Some(self.posting_list(tri, restrict.clone()));
                    } else {
                        list = Some(self.posting_and(list.unwrap(), tri, restrict.clone()));
                    }
                    if list.as_ref().unwrap().is_empty() {
                        return Vec::new();
                    }
                }
                
                let mut current_list = list;
                
                for sub in &q.sub {
                    let base = if current_list.is_none() { restrict.clone() } else { current_list.clone() };
                    current_list = Some(self.posting_query_rec(sub, base));
                    if current_list.as_ref().unwrap().is_empty() {
                         return Vec::new();
                    }
                }
                current_list.unwrap_or_else(|| {
                    if let Some(r) = restrict { r } else { (0..self.num_name as u32).collect() }
                })
            }
            QueryOp::Or => {
                 let mut list = None;
                 for t in &q.trigram {
                     let tri = trigram_u32(t);
                     if list.is_none() {
                         list = Some(self.posting_list(tri, restrict.clone()));
                     } else {
                         list = Some(self.posting_or(list.unwrap(), tri, restrict.clone()));
                     }
                 }
                 
                 let mut current_list = list.unwrap_or_default();
                 
                 for sub in &q.sub {
                     let list1 = self.posting_query_rec(sub, restrict.clone());
                     current_list = merge_or(current_list, list1);
                 }
                 current_list
            }
        }
    }
    
    fn posting_list(&self, trigram: u32, restrict: Option<Vec<u32>>) -> Vec<u32> {
        let mut r = PostReader::new(self, trigram, restrict);
        let mut x = Vec::with_capacity(r.max());
        while r.next() {
            x.push(r.fileid as u32);
        }
        x
    }
    
    fn posting_and(&self, list: Vec<u32>, trigram: u32, restrict: Option<Vec<u32>>) -> Vec<u32> {
        let mut r = PostReader::new(self, trigram, restrict);
        let mut x = Vec::new(); // Upper bound is list.len()
        let mut i = 0;
        while r.next() {
            let fileid = r.fileid as u32;
            while i < list.len() && list[i] < fileid {
                i += 1;
            }
            if i < list.len() && list[i] == fileid {
                x.push(fileid);
                i += 1;
            }
        }
        x
    }
    
    fn posting_or(&self, list: Vec<u32>, trigram: u32, restrict: Option<Vec<u32>>) -> Vec<u32> {
         let mut r = PostReader::new(self, trigram, restrict);
         let mut x = Vec::with_capacity(list.len() + r.max());
         let mut i = 0;
         while r.next() {
             let fileid = r.fileid as u32;
             while i < list.len() && list[i] < fileid {
                 x.push(list[i]);
                 i += 1;
             }
             x.push(fileid);
             if i < list.len() && list[i] == fileid {
                 i += 1;
             }
         }
         while i < list.len() {
             x.push(list[i]);
             i += 1;
         }
         x
    }
    
    fn find_list_v2(&self, trigram: u32) -> (usize, usize) {
        if self.num_post_block == 0 {
            return (0, 0);
        }
        
        let post_index_end = self.post_index + self.num_post_block * POST_BLOCK_SIZE;
        if post_index_end > self.mmap.len() {
            return (0, 0);
        }
        
        let b = &self.mmap[self.post_index .. post_index_end];
        
        let mut i = 0; 
        let mut j = self.num_post_block;
        while i < j {
             let h = i + (j - i) / 2;
             let off = h * POST_BLOCK_SIZE;
             if off + 3 > b.len() {
                 break;
             }
             let t = read_u24_be(&b[off..off+3]);
             if t > trigram {
                 j = h;
             } else {
                 i = h + 1;
             }
        }
        
        if i == 0 {
            return (0, 0);
        }
        
        let block_start = (i - 1) * POST_BLOCK_SIZE;
        let block_end = i * POST_BLOCK_SIZE;
        if block_end > b.len() {
            return (0, 0);
        }
        let mut block = &b[block_start .. block_end];
        
        let mut offset = 0;
        
        while block.len() >= 3 {
             let t = read_u24_be(&block[0..3]);
             if t == 0 {
                 break;
             }
             if block.len() < 3 {
                 break;
             }
             let (count, n1) = read_uvarint(&block[3..]);
             if n1 == 0 || 3 + n1 > block.len() {
                 break;
             }
             let (off, n2) = read_uvarint(&block[3+n1..]);
             if n2 == 0 || 3 + n1 + n2 > block.len() {
                 break;
             }
             offset += off as usize;
             
             if t == trigram {
                 return (count as usize, offset);
             }
             
             block = &block[3+n1+n2..];
        }
        (0, 0)
    }
}

fn trigram_u32(s: &str) -> u32 {
    let b = s.as_bytes();
    ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | (b[2] as u32)
}

fn merge_or(l1: Vec<u32>, l2: Vec<u32>) -> Vec<u32> {
    let mut l = Vec::with_capacity(l1.len() + l2.len());
    let mut i = 0;
    let mut j = 0;
    while i < l1.len() || j < l2.len() {
        if j == l2.len() || (i < l1.len() && l1[i] < l2[j]) {
            l.push(l1[i]);
            i += 1;
        } else if i == l1.len() || (j < l2.len() && l1[i] > l2[j]) {
            l.push(l2[j]);
            j += 1;
        } else {
            l.push(l1[i]);
            i += 1;
            j += 1;
        }
    }
    l
}

// Helpers

fn read_uvarint(buf: &[u8]) -> (u64, usize) {
    let mut x: u64 = 0;
    let mut s: u32 = 0;
    for (i, &b) in buf.iter().enumerate() {
        if b < 0x80 {
            if i > 9 || (i == 9 && b > 1) {
                return (0, 0); // overflow
            }
            return (x | ((b as u64) << s), i + 1);
        }
        x |= ((b & 0x7f) as u64) << s;
        s += 7;
    }
    (0, 0)
}

// PathReader

pub struct PathReader<'a> {
    data: &'a [u8],
    limit: usize,
    path: String,
}

impl<'a> PathReader<'a> {
    pub fn new(data: &'a [u8], limit: usize) -> Self {
        PathReader {
            data,
            limit,
            path: String::new(),
        }
    }
    
    pub fn next(&mut self) -> Option<String> {
        if self.limit == 0 { return None; }
        self.limit -= 1;
        
        let (pre, w1) = read_uvarint(self.data);
        if w1 == 0 { return None; }
        self.data = &self.data[w1..];
        
        let (n, w2) = read_uvarint(self.data);
        if w2 == 0 { return None; }
        self.data = &self.data[w2..];
        
        let pre = pre as usize;
        let n = n as usize;
        
        if pre > self.path.len() {
             return None;
        }
        self.path.truncate(pre);
        if n > self.data.len() {
            return None;
        }
        match str::from_utf8(&self.data[..n]) {
            Ok(s) => {
                self.path.push_str(s);
                self.data = &self.data[n..];
                Some(self.path.clone())
            }
            Err(_) => None,
        }
    }
}

pub struct PostMapIter<'a> {
    ix: &'a Index,
    block: &'a [u8],
    next_block: usize,
    tri_num: usize,
    file_offset: usize,
}

impl<'a> PostMapIter<'a> {
    fn new(ix: &'a Index) -> Self {
        PostMapIter {
            ix,
            block: &[],
            next_block: 0,
            tri_num: 0,
            file_offset: 0,
        }
    }
    
    // Returns (trigram, count, offset)
    pub fn next(&mut self) -> Option<(u32, usize, usize)> {
        if self.tri_num >= self.ix.num_post {
            return None;
        }
        
        self.tri_num += 1;
        
        if self.block.len() < 3 || (self.block[0] == 0 && self.block[1] == 0 && self.block[2] == 0) {
             if self.ix.post_index + self.next_block + POST_BLOCK_SIZE > self.ix.mmap.len() {
                 return None;
             }
             let start = self.ix.post_index + self.next_block;
             self.block = &self.ix.mmap[start .. start + POST_BLOCK_SIZE];
             self.next_block += POST_BLOCK_SIZE;
             self.file_offset = 0;
        }
        
        let trigram = read_u24_be(&self.block[0..3]);
        self.block = &self.block[3..];
        
        let (count, n1) = read_uvarint(self.block);
        self.block = &self.block[n1..];
        
        let (off, n2) = read_uvarint(self.block);
        self.block = &self.block[n2..];
        
        self.file_offset += off as usize;
        
        Some((trigram, count as usize, self.file_offset))
    }
}


// PostReader

pub struct PostReader<'a> {
    count: usize,
    // offset: usize, // not strictly needed if we just hold the slice
    pub fileid: i32,
    restrict: Option<Vec<u32>>,
    delta: DeltaReader<'a>,
}

impl<'a> PostReader<'a> {
    pub fn new(ix: &'a Index, trigram: u32, restrict: Option<Vec<u32>>) -> Self {
        let (count, offset) = ix.find_list_v2(trigram);
        if count == 0 {
             return PostReader {
                 count: 0,
                 fileid: -1,
                 restrict: None,
                 delta: DeltaReader::new(&[]),
             };
        }
        
        let data_start = ix.post_data + offset + 3;
        if data_start >= ix.mmap.len() {
            return PostReader {
                count: 0,
                fileid: -1,
                restrict: None,
                delta: DeltaReader::new(&[]),
            };
        }
        
        let data = ix.slice_from(data_start);
        
        PostReader {
            count,
            fileid: -1,
            restrict,
            delta: DeltaReader::new(data),
        }
    }
    
    pub fn max(&self) -> usize {
        self.count
    }
    
    pub fn next(&mut self) -> bool {
        if self.count == 0 {
            return false;
        }
        
        while self.count > 0 {
            self.count -= 1;
            let d = self.delta.next();
            if d.is_none() {
                // corrupt
                return false; 
            }
            let delta = d.unwrap();
            self.fileid += delta as i32;
            
            if let Some(ref mut rest) = self.restrict {
                 while !rest.is_empty() && (rest[0] as i32) < self.fileid {
                     rest.remove(0);
                 }
                 if rest.is_empty() || (rest[0] as i32) != self.fileid {
                     continue;
                 }
            }
            return true;
        }
        false
    }
}

// DeltaReader

pub struct DeltaReader<'a> {
    d: &'a [u8],
    b: u64,
    nb: u32,
}

impl<'a> DeltaReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        DeltaReader { d: data, b: 0, nb: 0 }
    }
    
    pub fn next(&mut self) -> Option<u32> {
        let i = self.next64()?;
        if i == DELTA_ZERO_ENC as u64 {
            Some(0)
        } else if i > DELTA_ZERO_ENC as u64 {
            Some((i - 1) as u32)
        } else {
            Some(i as u32)
        }
    }

    fn next64(&mut self) -> Option<u64> {
        let mut lg = 0;
        while self.b == 0 {
            if self.d.is_empty() { return None; }
            lg += self.nb;
            self.b = self.d[0] as u64;
            self.nb = 8;
            self.d = &self.d[1..];
        }
        
        let zeros = self.b.trailing_zeros();
        lg += zeros;
        self.b >>= zeros + 1;
        self.nb -= zeros + 1;
        
        let mut x = 1u64 << lg;
        let mut nb = 0;
        
        while self.nb < lg {
            x |= self.b << nb;
            nb += self.nb;
            lg -= self.nb;
            
            if self.d.is_empty() { return None; }
            self.b = self.d[0] as u64;
            self.nb = 8;
            self.d = &self.d[1..];
        }
        
        x |= (self.b & ((1 << lg) - 1)) << nb;
        self.b >>= lg;
        self.nb -= lg;
        
        Some(x)
    }
}
