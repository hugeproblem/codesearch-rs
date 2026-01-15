use std::fs::{self, File};
use std::io::{self, BufWriter, Write, Seek, SeekFrom, Read};
use std::cmp::{Ordering, min};
use std::collections::BinaryHeap;
use byteorder::{BigEndian, WriteBytesExt};
use memmap2::Mmap;
use crate::sparse_set::Set as SparseSet;

const NAME_GROUP_SIZE: usize = 16;
const MAX_FILE_LEN: u64 = 1 << 30;
const MAX_LINE_LEN: usize = 2000;
const MAX_TEXT_TRIGRAMS: usize = 20000;
const INVALID_TRIGRAM: u32 = (1 << 24) - 1;
const POST_BLOCK_SIZE: usize = 256;
const DELTA_ZERO_ENC: u32 = 16;
const WRITE_VERSION: i32 = 2;

// --- Buffer ---

pub struct IndexBuffer {
    file: File,
    writer: BufWriter<File>,
    offset: u64,
}

impl IndexBuffer {
    pub fn new(name: &str) -> io::Result<Self> {
        // println!("IndexBuffer::new({})", name);
        let file = if name.is_empty() {
            tempfile::tempfile()?
        } else {
            fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(name)?
        };
        let writer = BufWriter::with_capacity(256 * 1024, file.try_clone()?);
        
        Ok(IndexBuffer {
            file,
            writer,
            offset: 0,
        })
    }

    pub fn write_byte(&mut self, b: u8) -> io::Result<()> {
        self.writer.write_all(&[b])?;
        self.offset += 1;
        Ok(())
    }

    pub fn write_bytes(&mut self, b: &[u8]) -> io::Result<()> {
        self.writer.write_all(b)?;
        self.offset += b.len() as u64;
        Ok(())
    }

    pub fn write_string(&mut self, s: &str) -> io::Result<()> {
        self.write_bytes(s.as_bytes())
    }

    pub fn write_trigram(&mut self, t: u32) -> io::Result<()> {
        self.write_byte((t >> 16) as u8)?;
        self.write_byte((t >> 8) as u8)?;
        self.write_byte(t as u8)
    }

    pub fn write_uvarint(&mut self, x: u64) -> io::Result<()> {
        let mut buf = [0u8; 10];
        let mut n = 0;
        let mut v = x;
        loop {
            let mut byte = (v & 0x7F) as u8;
            v >>= 7;
            if v != 0 {
                byte |= 0x80;
            }
            buf[n] = byte;
            n += 1;
            if v == 0 {
                break;
            }
        }
        self.write_bytes(&buf[..n])
    }

    pub fn write_uint32(&mut self, x: u32) -> io::Result<()> {
        self.writer.write_u32::<BigEndian>(x)?;
        self.offset += 4;
        Ok(())
    }

    pub fn write_uint64(&mut self, x: u64) -> io::Result<()> {
        self.writer.write_u64::<BigEndian>(x)?;
        self.offset += 8;
        Ok(())
    }

    pub fn offset(&self) -> u64 {
        self.offset
    }

    pub fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }

    pub fn finish(mut self) -> io::Result<File> {
        self.flush()?;
        let mut f = self.file;
        f.seek(SeekFrom::Start(0))?;
        Ok(f)
    }
    
    pub fn align(&mut self, n: u64) -> io::Result<()> {
        if WRITE_VERSION == 1 {
            return Ok(());
        }
        let off = self.offset;
        if off % n == 0 {
            return Ok(());
        }
        let pad = n - (off % n);
        for _ in 0..pad {
            self.write_byte(0)?;
        }
        Ok(())
    }
}

// --- Delta Encoding ---

pub struct DeltaWriter {
    nb: u32, 
    b: u8,   
}

impl DeltaWriter {
    pub fn new() -> Self {
        DeltaWriter { nb: 0, b: 0 }
    }

    fn write_bits(&mut self, w_out: &mut IndexBuffer, mut x: u32, mut n: u32) -> io::Result<()> {
        while n > 0 {
            let space = 8 - self.nb;
            let mut w = n;
            if w > space {
                w = space;
            }
            self.b |= ((x & ((1 << w) - 1)) as u8) << self.nb;
            x >>= w;
            self.nb += w;
            n -= w;
            if self.nb == 8 {
                w_out.write_byte(self.b)?;
                self.b = 0;
                self.nb = 0;
            }
        }
        Ok(())
    }

    pub fn write(&mut self, w_out: &mut IndexBuffer, mut x: u32) -> io::Result<()> {
        if x == 0 {
            x = DELTA_ZERO_ENC;
        } else if x >= DELTA_ZERO_ENC {
            x += 1;
        }
        
        let lg = 31 - x.leading_zeros(); 
        let val = x & ((1 << lg) - 1);
        
        self.write_bits(w_out, 1 << lg, lg + 1)?;
        self.write_bits(w_out, val, lg)
    }
    
    pub fn finish(&mut self, w_out: &mut IndexBuffer) -> io::Result<()> {
        if self.nb > 0 {
            w_out.write_byte(self.b)?;
            self.nb = 0;
            self.b = 0;
        }
        Ok(())
    }
}

pub struct DeltaReader<'a> {
    d: &'a [u8],
    b: u64,
    nb: u32,
}

impl<'a> DeltaReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        DeltaReader { d: data, b: 0, nb: 0 }
    }
    
    fn clear_bits(&mut self) {
        self.b = 0;
        self.nb = 0;
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


// --- Path ---

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexPath {
    pub s: String,
}

impl IndexPath {
    pub fn new(s: String) -> Self {
        IndexPath { s }
    }
}

impl PartialOrd for IndexPath {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for IndexPath {
    fn cmp(&self, other: &Self) -> Ordering {
        let a = self.s.as_bytes();
        let b = other.s.as_bytes();
        let len = min(a.len(), b.len());
        for i in 0..len {
            let mut ai = a[i];
            let mut bi = b[i];
            if ai == b'/' { ai = 0; }
            if bi == b'/' { bi = 0; }
            if ai != bi {
                return ai.cmp(&bi);
            }
        }
        a.len().cmp(&b.len())
    }
}

pub struct PathWriterState {
    last: String,
    n: usize,
    group: usize,
    start: Option<u64>,  // Track the starting offset
}

impl PathWriterState {
    pub fn new(group: usize) -> Self {
        PathWriterState { last: String::new(), n: 0, group, start: None }
    }
}

pub struct PathWriter<'a> {
    data: &'a mut IndexBuffer,
    index: Option<&'a mut IndexBuffer>,
    state: &'a mut PathWriterState,
    start: u64,
}

impl<'a> PathWriter<'a> {
    pub fn new(data: &'a mut IndexBuffer, index: Option<&'a mut IndexBuffer>, state: &'a mut PathWriterState) -> Self {
        // Only set start on first call
        if state.start.is_none() {
            state.start = Some(data.offset());
        }
        let start = state.start.unwrap();
        PathWriter {
            data,
            index,
            state,
            start,
        }
    }

    pub fn write(&mut self, p: &IndexPath) -> io::Result<()> {
        let write_index = (self.state.group == 0 && self.state.n == 0) || (self.state.group > 0 && self.state.n % self.state.group == 0);
        
        if write_index {
             if let Some(ref mut idx) = self.index {
                 // Write 8-byte offset for compatibility with reader
                 let off = self.data.offset() - self.start;
                 idx.write_uint64(off)?;
             }
        }
        
        let mut pre = 0;
        if !write_index {
             let ls = self.state.last.as_bytes();
             let ps = p.s.as_bytes();
             while pre < ls.len() && pre < ps.len() && ls[pre] == ps[pre] {
                 pre += 1;
             }
        }
        
        self.data.write_uvarint(pre as u64)?;
        self.data.write_uvarint((p.s.len() - pre) as u64)?;
        self.data.write_string(&p.s[pre..])?;
        self.state.last = p.s.clone();
        self.state.n += 1;
        Ok(())
    }
}


// --- Post Data ---

pub struct PostDataWriter<'a> {
    out: &'a mut IndexBuffer,
    post_index: Option<&'a mut IndexBuffer>,
    base: u64,
    last_offset: u64,
    offset: u64,
    last_id: i32,
    t: u32,
    delta: DeltaWriter,
    pub num_trigram: usize,
    count: usize, // number of files for current trigram
    block: Vec<u8>,
}

impl<'a> PostDataWriter<'a> {
    pub fn new(out: &'a mut IndexBuffer, post_index: Option<&'a mut IndexBuffer>) -> Self {
        let base = out.offset();
        PostDataWriter {
            out,
            post_index,
            base,
            last_offset: base,
            offset: 0,
            last_id: -1,
            t: 0,
            delta: DeltaWriter::new(),
            num_trigram: 0,
            count: 0,
            block: Vec::with_capacity(POST_BLOCK_SIZE),
        }
    }
    
    pub fn trigram(&mut self, t: u32) -> io::Result<()> {
        if t == 0 { panic!("invalid trigram"); }
        self.offset = self.out.offset();
        self.t = t;
        self.last_id = -1;
        self.count = 0;
        self.num_trigram += 1;
        self.out.write_trigram(t)
    }
    
    pub fn fileid(&mut self, id: i32) -> io::Result<()> {
        let diff = id - self.last_id;
        self.delta.write(self.out, diff as u32)?;
        self.last_id = id;
        self.count += 1;
        Ok(())
    }
    
    pub fn end_trigram(&mut self) -> io::Result<()> {
        self.delta.write(self.out, 0)?;
        self.delta.finish(self.out)?;
        
        if let Some(ref mut idx) = self.post_index {
             let mut buf = [0u8; 3 + 10 + 10 + 10];
             buf[0] = (self.t >> 16) as u8;
             buf[1] = (self.t >> 8) as u8;
             buf[2] = self.t as u8;
             
             let mut n = 3;
             let append_varint = |val: u64, dest: &mut [u8], pos: &mut usize| {
                  let mut v = val;
                  loop {
                      let mut byte = (v & 0x7F) as u8;
                      v >>= 7;
                      if v != 0 { byte |= 0x80; }
                      dest[*pos] = byte;
                      *pos += 1;
                      if v == 0 { break; }
                  }
             };
             
             append_varint(self.count as u64, &mut buf, &mut n);
             
             let n1_start = n;
             append_varint(self.offset - self.last_offset, &mut buf, &mut n);
             let _n1_len = n - n1_start;
             
             if self.block.len() + n > POST_BLOCK_SIZE {
                 self.block.resize(POST_BLOCK_SIZE, 0);
                 idx.write_bytes(&self.block)?;
                 self.block.clear();
                 // Recalculate with base offset
                 n = n1_start;
                 append_varint(self.offset - self.base, &mut buf, &mut n);
             }
             
             self.block.extend_from_slice(&buf[..n]);
             self.last_offset = self.offset;
        }
        Ok(())
    }
    
    pub fn flush(&mut self) -> io::Result<()> {
        if let Some(ref mut idx) = self.post_index {
            if !self.block.is_empty() {
                self.block.resize(POST_BLOCK_SIZE, 0);
                idx.write_bytes(&self.block)?;
                self.block.clear();
            }
        }
        Ok(())
    }
}

// --- Merging ---

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PostEntry(u64);

impl PostEntry {
    pub fn new(trigram: u32, fileid: i32) -> Self {
        PostEntry((trigram as u64) << 40 | (fileid as u64))
    }
    
    pub fn trigram(&self) -> u32 {
        (self.0 >> 40) as u32
    }
    
    pub fn fileid(&self) -> i32 {
        (self.0 & 0xFFFFFFFFFF) as i32
    }
}

impl PartialOrd for PostEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PostEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}


pub struct AllPostReader<'a> {
    trigram: u32,
    fileid: i32,
    delta: DeltaReader<'a>,
}

impl<'a> AllPostReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        AllPostReader {
            trigram: INVALID_TRIGRAM,
            fileid: -1,
            delta: DeltaReader::new(data),
        }
    }
    
    pub fn next(&mut self) -> Option<PostEntry> {
        loop {
            if self.trigram == INVALID_TRIGRAM {
                 if self.delta.d.len() < 3 {
                     if self.delta.d.is_empty() { return None; }
                     panic!("invalid temporary file");
                 }
                 self.trigram = (self.delta.d[0] as u32) << 16 | (self.delta.d[1] as u32) << 8 | (self.delta.d[2] as u32);
                 self.delta.d = &self.delta.d[3..];
                 self.fileid = -1;
                 self.delta.clear_bits();
            }
            
            let delta = self.delta.next()?; // calls next64
            if delta == 0 {
                self.delta.clear_bits();
                self.trigram = INVALID_TRIGRAM;
                continue;
            }
            self.fileid += delta as i32;
            return Some(PostEntry::new(self.trigram, self.fileid));
        }
    }
}

// Helpers for Heap
struct HeapItem {
    entry: PostEntry,
    reader_idx: usize,
}

impl PartialEq for HeapItem {
    fn eq(&self, other: &Self) -> bool {
        self.entry == other.entry
    }
}

impl Eq for HeapItem {}

impl PartialOrd for HeapItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HeapItem {
    fn cmp(&self, other: &Self) -> Ordering {
        other.entry.cmp(&self.entry) // Reverse for MinHeap
    }
}


// --- IndexWriter ---

pub struct IndexWriter {
    trigram: SparseSet,
    post: Vec<PostEntry>,
    
    // Buffers as Options to take ownership in flush
    name_buf: Option<IndexBuffer>,
    post_buf: Option<IndexBuffer>,
    name_index_buf: Option<IndexBuffer>,
    post_index_buf: Option<IndexBuffer>,
    
    main_buf: IndexBuffer, 
    
    num_name: usize,
    num_trigram: usize,
    total_bytes: i64,
    
    post_ends: Vec<u64>, 
    
    pub verbose: bool,
    pub log_skip: bool,
    
    // State
    name_writer_state: PathWriterState,
}

impl IndexWriter {
    pub fn create(file: &str) -> io::Result<Self> {
        let name_buf = IndexBuffer::new("")?;
        let post_buf = IndexBuffer::new("")?;
        let name_index_buf = IndexBuffer::new("")?;
        let post_index_buf = IndexBuffer::new("")?;
        let main_buf = IndexBuffer::new(file)?;
        
        Ok(IndexWriter {
            trigram: SparseSet::new(1 << 24),
            post: Vec::with_capacity(256 * 1024), 
            name_buf: Some(name_buf),
            post_buf: Some(post_buf),
            name_index_buf: Some(name_index_buf),
            post_index_buf: Some(post_index_buf),
            main_buf,
            num_name: 0,
            num_trigram: 0,
            total_bytes: 0,
            post_ends: Vec::new(),
            verbose: false,
            log_skip: false,
            name_writer_state: PathWriterState::new(NAME_GROUP_SIZE),
        })
    }
    
    pub fn add_file(&mut self, name: &str) -> io::Result<()> {
        let f = File::open(name);
        if f.is_err() {
            if self.log_skip { eprintln!("{}: {}", name, f.err().unwrap()); }
            return Ok(());
        }
        let mut f = f.unwrap();
        let len = f.metadata()?.len();
        if len > MAX_FILE_LEN {
             if self.log_skip { eprintln!("{}: too long, ignoring", name); }
             return Ok(());
        }
        
        let mut buf = Vec::with_capacity(len as usize + 1);
        f.read_to_end(&mut buf)?;
        
        self.trigram.reset();
        let mut tv: u32 = 0;
        let mut n = 0;
        let mut linelen = 0;
        
        for &c in &buf {
            tv = (tv << 8) & 0xFFFFFF;
            tv |= c as u32;
            n += 1;
            if n >= 3 {
                self.trigram.add(tv);
            }
            if c == 0 {
                if self.log_skip { eprintln!("{}: contains NUL, ignoring", name); }
                return Ok(());
            }
            // Note: We don't validate UTF-8 here as many source files use Latin-1 or other encodings.
            // The NUL check above is sufficient to skip binary files.
            if linelen > MAX_LINE_LEN {
                 if self.log_skip { eprintln!("{}: very long lines, ignoring", name); }
                 return Ok(());
            }
            linelen += 1;
            if c == b'\n' { linelen = 0; }
        }
        
        if self.trigram.len() > MAX_TEXT_TRIGRAMS {
            if self.log_skip { eprintln!("{}: too many trigrams, ignoring", name); }
            return Ok(());
        }
        
        self.total_bytes += len as i64;
        let fileid = self.add_name(name)?;
        
        let trigrams = self.trigram.dense().to_vec();
        if self.verbose {
            println!("DEBUG: File {} added {} trigrams", name, trigrams.len());
        }
        for trigram in trigrams {
            if self.post.len() >= self.post.capacity() {
                self.flush_post()?;
            }
            self.post.push(PostEntry::new(trigram, fileid as i32));
        }
        
        Ok(())
    }
    
    fn add_name(&mut self, name: &str) -> io::Result<usize> {
        let id = self.num_name;
        
        let mut writer = PathWriter::new(
            self.name_buf.as_mut().unwrap(),
            self.name_index_buf.as_mut().map(|b| b),
            &mut self.name_writer_state
        );
        writer.write(&IndexPath::new(name.to_string()))?;
        
        self.num_name += 1;
        Ok(id)
    }
    
    fn flush_post(&mut self) -> io::Result<()> {
        self.post.sort();
        if self.verbose {
            println!("DEBUG: flush_post sorted {} entries", self.post.len());
        }
        
        let mut w = PostDataWriter::new(self.post_buf.as_mut().unwrap(), None); 
        
        let mut i = 0;
        while i < self.post.len() {
            let t = self.post[i].trigram();
            w.trigram(t)?;
            while i < self.post.len() && self.post[i].trigram() == t {
                w.fileid(self.post[i].fileid())?;
                i += 1;
            }
            w.end_trigram()?; 
        }
        w.flush()?;
        
        if self.verbose {
            println!("DEBUG: flush_post wrote {} trigrams", w.num_trigram);
        }
        
        self.post.clear();
        let end = self.post_buf.as_ref().unwrap().offset();
        self.post_ends.push(end);
        Ok(())
    }
    
    pub fn flush(&mut self) -> io::Result<()> {
        self.flush_post()?;
        
        self.main_buf.write_string("csearch index 2\n")?;
        
        let roots_off = self.main_buf.offset();
        let roots_count = 0; 
        self.main_buf.align(16)?;
        
        let name_off = self.main_buf.offset();
        let mut name_f = self.name_buf.take().unwrap().finish()?;
        let n = io::copy(&mut name_f, &mut self.main_buf.writer)?; 
        self.main_buf.offset += n;
        let name_count = self.num_name;
        self.main_buf.align(16)?;
        
        let post_off = self.main_buf.offset();
        self.merge_post()?;
        if self.verbose {
            println!("DEBUG: merge_post finished with num_trigram={}", self.num_trigram);
        }
        let trigram_count = self.num_trigram;
        self.main_buf.align(16)?;
        
        let name_idx_off = self.main_buf.offset();
        let mut name_idx_f = self.name_index_buf.take().unwrap().finish()?;
        let n = io::copy(&mut name_idx_f, &mut self.main_buf.writer)?;
        self.main_buf.offset += n;
        self.main_buf.align(16)?;
        
        let post_idx_off = self.main_buf.offset();
        let mut post_idx_f = self.post_index_buf.take().unwrap().finish()?;
        let n = io::copy(&mut post_idx_f, &mut self.main_buf.writer)?;
        self.main_buf.offset += n;
        
        self.main_buf.write_uint64(roots_off)?;
        self.main_buf.write_uint64(roots_count as u64)?;
        self.main_buf.write_uint64(name_off)?;
        self.main_buf.write_uint64(name_count as u64)?;
        self.main_buf.write_uint64(post_off)?;
        self.main_buf.write_uint64(trigram_count as u64)?;
        self.main_buf.write_uint64(name_idx_off)?;
        self.main_buf.write_uint64(post_idx_off)?;
        self.main_buf.write_string("\ncsearch trlr 2\n")?;
        
        self.main_buf.flush()?;
        
        Ok(())
    }
    
    fn merge_post(&mut self) -> io::Result<()> {
        let post_file = self.post_buf.take().unwrap().finish()?;
        let mmap = unsafe { Mmap::map(&post_file)? };
        
        let mut readers = Vec::new();
        let mut start = 0;
        for &end in &self.post_ends {
            readers.push(AllPostReader::new(&mmap[start as usize..end as usize]));
            start = end;
        }
        
        let mut heap = BinaryHeap::new();
        for (i, r) in readers.iter_mut().enumerate() {
            if let Some(entry) = r.next() {
                heap.push(HeapItem { entry, reader_idx: i });
            }
        }
        
        // We need to write to main_buf, and also update post_index_buf.
        // But post_index_buf is currently an Option<IndexBuffer> in self.
        // It must be Some.
        // However, self is mut borrowed.
        // PostDataWriter needs &mut main_buf and Option<&mut post_index_buf>.
        
        // Let's take post_index_buf out temporarily? 
        // No, I can borrow it.
        // But PostDataWriter takes `Option<&'a mut IndexBuffer>`.
        // `self.post_index_buf.as_mut()` is `Option<&mut IndexBuffer>`.
        // But I need to convert `Option<Option<&mut IB>>` to `Option<&mut IB>`.
        // `self.post_index_buf.as_mut().map(|b| b)`?
        
        // Wait, `PostDataWriter::new` takes `post_index: Option<&'a mut IndexBuffer>`.
        // `self.post_index_buf.as_mut()` gives `Option<&mut IndexBuffer>`.
        // So I can pass `self.post_index_buf.as_mut()`.
        
        // But `w` borrows `self.main_buf` and `self.post_index_buf`.
        // That splits the borrow of `self`.
        // This is fine as long as fields are disjoint.
        // But `self` is fully borrowed if I don't split.
        
        // Rust borrow checker might complain if I call `PostDataWriter::new(&mut self.main_buf, self.post_index_buf.as_mut())`.
        // Because `self.main_buf` and `self.post_index_buf` are fields of `self`.
        // I can destructure or borrow disjointly.
        
        // But I can't easily destructure in method.
        // I can do:
        let main_buf = &mut self.main_buf;
        let post_index_buf = self.post_index_buf.as_mut(); // This might panic if I took it? 
        // I haven't taken post_index_buf yet. I take it in flush AFTER merge_post.
        
        let mut w = PostDataWriter::new(main_buf, post_index_buf);
        
        while let Some(item) = heap.pop() {
            let t = item.entry.trigram();
            w.trigram(t)?;
            w.fileid(item.entry.fileid())?;
            
            // Advance reader
            if let Some(next_entry) = readers[item.reader_idx].next() {
                heap.push(HeapItem { entry: next_entry, reader_idx: item.reader_idx });
            }
            
            // Process other entries with same trigram
            loop {
                let peek = heap.peek();
                if peek.is_none() { break; }
                let p = peek.unwrap();
                if p.entry.trigram() != t { break; }
                
                // Must pop
                let item = heap.pop().unwrap();
                w.fileid(item.entry.fileid())?;
                
                if let Some(next_entry) = readers[item.reader_idx].next() {
                    heap.push(HeapItem { entry: next_entry, reader_idx: item.reader_idx });
                }
            }
            
            w.end_trigram()?;
        }
        w.flush()?;
        self.num_trigram = w.num_trigram;
        Ok(())
    }
}



