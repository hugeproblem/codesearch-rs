pub struct Set {
    dense: Vec<u32>,
    sparse: Vec<u32>,
}

impl Set {
    pub fn new(max: u32) -> Self {
        Self {
            dense: Vec::new(),
            sparse: vec![0; max as usize],
        }
    }

    pub fn reset(&mut self) {
        self.dense.clear();
    }

    pub fn add(&mut self, x: u32) {
        let idx = x as usize;
        if idx >= self.sparse.len() {
            return;
        }
        let v = self.sparse[idx];
        if (v as usize) < self.dense.len() && self.dense[v as usize] == x {
            return;
        }
        let n = self.dense.len();
        self.sparse[idx] = n as u32;
        self.dense.push(x);
    }

    pub fn has(&self, x: u32) -> bool {
        let idx = x as usize;
        if idx >= self.sparse.len() {
            return false;
        }
        let v = self.sparse[idx];
        (v as usize) < self.dense.len() && self.dense[v as usize] == x
    }

    pub fn dense(&self) -> &[u32] {
        &self.dense
    }

    pub fn len(&self) -> usize {
        self.dense.len()
    }

    pub fn is_empty(&self) -> bool {
        self.dense.is_empty()
    }
}
