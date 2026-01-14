use regex_syntax::hir::{Hir, HirKind, Class};
use std::cmp::Ordering;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryOp {
    All,
    None,
    And,
    Or,
}

#[derive(Debug, Clone)]
pub struct Query {
    pub op: QueryOp,
    pub trigram: Vec<String>, // Sorted and unique
    pub sub: Vec<Query>,
}

impl Query {
    pub fn all() -> Self {
        Query { op: QueryOp::All, trigram: Vec::new(), sub: Vec::new() }
    }

    pub fn none() -> Self {
        Query { op: QueryOp::None, trigram: Vec::new(), sub: Vec::new() }
    }

    pub fn and(self, other: Query) -> Query {
        self.and_or(other, QueryOp::And)
    }

    pub fn or(self, other: Query) -> Query {
        self.and_or(other, QueryOp::Or)
    }

    fn and_or(mut self, mut other: Query, op: QueryOp) -> Query {
        if self.trigram.is_empty() && self.sub.len() == 1 {
            self = self.sub.into_iter().next().unwrap();
        }
        if other.trigram.is_empty() && other.sub.len() == 1 {
            other = other.sub.into_iter().next().unwrap();
        }

        if self.implies(&other) {
            if op == QueryOp::And {
                return self;
            }
            return other;
        }
        if other.implies(&self) {
            if op == QueryOp::And {
                return other;
            }
            return self;
        }

        let q_atom = self.trigram.len() == 1 && self.sub.is_empty();
        let r_atom = other.trigram.len() == 1 && other.sub.is_empty();

        if self.op == op && (other.op == op || r_atom) {
            self.trigram = union_sets(self.trigram, other.trigram);
            self.sub.extend(other.sub);
            return self;
        }
        if other.op == op && q_atom {
            other.trigram = union_sets(other.trigram, self.trigram);
            return other;
        }
        if q_atom && r_atom {
            let mut q = self;
            q.op = op;
            q.trigram.extend(other.trigram);
            return q;
        }

        if self.op == op {
            self.sub.push(other);
            return self;
        }
        if other.op == op {
            other.sub.push(self);
            return other;
        }

        // Factor out common trigrams
        let (common, q_trig, r_trig) = intersection_split(self.trigram, other.trigram);
        self.trigram = q_trig;
        other.trigram = r_trig;

        if !common.is_empty() {
            let s = self.and_or(other, op);
            let other_op = if op == QueryOp::And { QueryOp::Or } else { QueryOp::And };
            let t = Query { op: other_op, trigram: common, sub: Vec::new() };
            return t.and_or(s, other_op);
        }

        Query {
            op,
            trigram: Vec::new(),
            sub: vec![self, other],
        }
    }

    pub fn implies(&self, other: &Query) -> bool {
        if self.op == QueryOp::None || other.op == QueryOp::All {
            return true;
        }
        if self.op == QueryOp::All || other.op == QueryOp::None {
            return false;
        }

        if self.op == QueryOp::And || (self.op == QueryOp::Or && self.trigram.len() == 1 && self.sub.is_empty()) {
            return trigrams_imply(&self.trigram, other);
        }

        if self.op == QueryOp::Or && other.op == QueryOp::Or &&
           !self.trigram.is_empty() && self.sub.is_empty() &&
           is_subset(&self.trigram, &other.trigram) {
            return true;
        }
        false
    }
    
    pub fn maybe_rewrite(&mut self, op: QueryOp) {
        if self.op != QueryOp::And && self.op != QueryOp::Or {
            return;
        }
        let n = self.sub.len() + self.trigram.len();
        if n > 1 {
            return;
        }
        if n == 0 {
            if self.op == QueryOp::And {
                self.op = QueryOp::All;
            } else {
                self.op = QueryOp::None;
            }
            return;
        }
        if self.sub.len() == 1 {
             let sub = self.sub.pop().unwrap();
             *self = sub;
             return;
        }
        self.op = op;
    }

    pub fn and_trigrams(self, t: Vec<String>) -> Query {
         if min_len(&t) < 3 {
             return self;
         }
         let mut or_q = Query::none();
         for tt in t {
             let mut trig = Vec::new();
             for i in 0..=tt.len().saturating_sub(3) {
                 if i + 3 <= tt.len() {
                    trig.push(tt[i..i+3].to_string());
                 }
             }
             clean_set(&mut trig);
             or_q = or_q.or(Query { op: QueryOp::And, trigram: trig, sub: Vec::new() });
         }
         self.and(or_q)
    }
}

fn trigrams_imply(t: &[String], q: &Query) -> bool {
    match q.op {
        QueryOp::Or => {
            for sub in &q.sub {
                if trigrams_imply(t, sub) {
                    return true;
                }
            }
            for i in 0..t.len() {
                if is_subset(&t[i..i+1], &q.trigram) {
                    return true;
                }
            }
            false
        }
        QueryOp::And => {
            for sub in &q.sub {
                if !trigrams_imply(t, sub) {
                    return false;
                }
            }
            if !is_subset(&q.trigram, t) {
                return false;
            }
            true
        }
        _ => false,
    }
}

// Set helpers
fn union_sets(mut s: Vec<String>, t: Vec<String>) -> Vec<String> {
    s.extend(t);
    clean_set(&mut s);
    s
}

fn cross_sets(s: &[String], t: &[String]) -> Vec<String> {
    let mut p = Vec::new();
    for ss in s {
        for tt in t {
            p.push(format!("{}{}", ss, tt));
        }
    }
    clean_set(&mut p);
    p
}

fn clean_set(s: &mut Vec<String>) {
    s.sort();
    s.dedup();
}

fn is_subset(s: &[String], t: &[String]) -> bool {
    let mut j = 0;
    for ss in s {
        while j < t.len() && &t[j] < ss {
            j += 1;
        }
        if j >= t.len() || &t[j] != ss {
            return false;
        }
    }
    true
}

fn intersection_split(s: Vec<String>, t: Vec<String>) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut common = Vec::new();
    let mut s_only = Vec::new();
    let mut t_only = Vec::new();
    let mut i = 0;
    let mut j = 0;
    while i < s.len() && j < t.len() {
        match s[i].cmp(&t[j]) {
            Ordering::Less => {
                s_only.push(s[i].clone());
                i += 1;
            }
            Ordering::Greater => {
                t_only.push(t[j].clone());
                j += 1;
            }
            Ordering::Equal => {
                common.push(s[i].clone());
                i += 1;
                j += 1;
            }
        }
    }
    while i < s.len() {
        s_only.push(s[i].clone());
        i += 1;
    }
    while j < t.len() {
        t_only.push(t[j].clone());
        j += 1;
    }
    (common, s_only, t_only)
}

fn min_len(s: &[String]) -> usize {
    if s.is_empty() {
        return 0;
    }
    s.iter().map(|x| x.len()).min().unwrap_or(0)
}

// Regex Analysis

const MAX_EXACT: usize = 7;
const MAX_SET: usize = 20;

#[derive(Clone, Debug)]
struct RegexpInfo {
    can_empty: bool,
    exact: Option<Vec<String>>,
    prefix: Vec<String>,
    suffix: Vec<String>,
    match_q: Query,
}

impl RegexpInfo {
    fn new() -> Self {
        RegexpInfo {
            can_empty: false,
            exact: None,
            prefix: Vec::new(),
            suffix: Vec::new(),
            match_q: Query::all(),
        }
    }
    
    fn any_match() -> Self {
        RegexpInfo {
            can_empty: true,
            exact: None,
            prefix: vec![String::new()],
            suffix: vec![String::new()],
            match_q: Query::all(),
        }
    }
    
    fn any_char() -> Self {
        RegexpInfo {
            can_empty: false,
            exact: None,
            prefix: vec![String::new()],
            suffix: vec![String::new()],
            match_q: Query::all(),
        }
    }

    fn no_match() -> Self {
         RegexpInfo {
            can_empty: false,
            exact: None,
            prefix: Vec::new(),
            suffix: Vec::new(),
            match_q: Query::none(),
        }
    }
    
    fn empty_string() -> Self {
        RegexpInfo {
            can_empty: true,
            exact: Some(vec![String::new()]),
            prefix: Vec::new(),
            suffix: Vec::new(),
            match_q: Query::all(),
        }
    }

    fn add_exact(&mut self) {
        if let Some(ref exact) = self.exact {
             self.match_q = self.match_q.clone().and_trigrams(exact.clone());
        }
    }

    fn simplify(&mut self, force: bool) {
        if let Some(mut exact) = self.exact.take() {
             clean_set(&mut exact);
             let min_l = min_len(&exact);
             if exact.len() > MAX_EXACT || (min_l >= 3 && force) || min_l >= 4 {
                 self.match_q = self.match_q.clone().and_trigrams(exact.clone());
                 for s in exact.iter() {
                     let n = s.len();
                     if n < 3 {
                         self.prefix.push(s.clone());
                         self.suffix.push(s.clone());
                     } else {
                         self.prefix.push(s[..2].to_string());
                         self.suffix.push(s[n-2..].to_string());
                     }
                 }
                 self.exact = None;
             } else {
                 self.exact = Some(exact);
             }
        }
        
        if self.exact.is_none() {
            simplify_set(&mut self.prefix, false);
            simplify_set(&mut self.suffix, true);
            self.match_q = self.match_q.clone().and_trigrams(self.prefix.clone());
            self.match_q = self.match_q.clone().and_trigrams(self.suffix.clone());
        }
    }
}

fn simplify_set(s: &mut Vec<String>, is_suffix: bool) {
    clean_set(s);
    
    let mut n = 3;
    while n == 3 || s.len() > MAX_SET {
        if n == 0 { break; } 
        
        let mut new_s = Vec::new();
        for str in s.iter() {
            let mut val = str.clone();
            if val.len() >= n {
                if !is_suffix {
                    val = val[..n-1].to_string();
                } else {
                    val = val[val.len()-n+1..].to_string();
                }
            }
            new_s.push(val);
        }
        *s = new_s;
        clean_set(s);
        
        n -= 1;
    }
    
    if is_suffix {
        s.sort_by(|a, b| {
            let ra: String = a.chars().rev().collect();
            let rb: String = b.chars().rev().collect();
            ra.cmp(&rb)
        });
    } else {
        s.sort();
    }
    
    let mut w = 0;
    let mut new_s = Vec::new();
    for str in s.iter() {
        if w == 0 {
             new_s.push(str.clone());
             w += 1;
             continue;
        }
        let prev = &new_s[w-1];
        let redundant = if is_suffix {
            str.ends_with(prev)
        } else {
            str.starts_with(prev)
        };
        
        if !redundant {
            new_s.push(str.clone());
            w += 1;
        }
    }
    *s = new_s;
}

pub fn analyze_regexp(pattern: &str) -> Result<Query, regex_syntax::Error> {
    let hir = regex_syntax::Parser::new().parse(pattern)?;
    let mut info = analyze_hir(&hir);
    info.simplify(true);
    info.add_exact();
    Ok(info.match_q)
}

fn analyze_hir(hir: &Hir) -> RegexpInfo {
    let mut info = match hir.kind() {
        HirKind::Empty => RegexpInfo::empty_string(),
        HirKind::Literal(lit) => {
            // regex-syntax Literal is bytes.
            // We assume UTF-8.
             match String::from_utf8(lit.0.to_vec()) {
                 Ok(s) => {
                     let mut info = RegexpInfo::new();
                     info.exact = Some(vec![s]);
                     info.match_q = Query::all();
                     info
                 }
                 Err(_) => RegexpInfo::any_char(), // Fallback for invalid UTF-8
             }
        }
        HirKind::Class(cls) => {
            // Handle character class
            let mut info = RegexpInfo::new();
            info.match_q = Query::all();
            
            let mut chars = Vec::new();
            match cls {
                Class::Unicode(u) => {
                     for range in u.ranges() {
                         let start = range.start();
                         let end = range.end();
                         // Count how many chars
                         // If too many, abort
                         let count = (end as u32) - (start as u32) + 1;
                         if chars.len() as u32 + count > 100 {
                             return RegexpInfo::any_char();
                         }
                         for c in start..=end {
                             chars.push(c);
                         }
                     }
                }
                Class::Bytes(b) => {
                     for range in b.ranges() {
                         let start = range.start();
                         let end = range.end();
                         let count = (end as u16) - (start as u16) + 1;
                         if chars.len() as u16 + count > 100 {
                             return RegexpInfo::any_char();
                         }
                         for c in start..=end {
                             chars.push(c as char);
                         }
                     }
                }
            }
            if chars.is_empty() {
                return RegexpInfo::no_match();
            }
            let mut exact = Vec::new();
            for c in chars {
                exact.push(c.to_string());
            }
            info.exact = Some(exact);
            info
        }
        HirKind::Look(_) => RegexpInfo::empty_string(),
        HirKind::Repetition(rep) => {
             if rep.min == 0 {
                 RegexpInfo::any_match()
             } else {
                 // Plus (min >= 1)
                 let mut sub_info = analyze_hir(&rep.sub);
                 if let Some(exact) = sub_info.exact {
                     sub_info.prefix = exact.clone();
                     sub_info.suffix = exact;
                     sub_info.exact = None;
                 }
                 sub_info
             }
        }
        HirKind::Capture(cap) => analyze_hir(&cap.sub),
        HirKind::Concat(subs) => {
            fold(concat_info, subs, RegexpInfo::empty_string())
        }
        HirKind::Alternation(subs) => {
            fold(alternate_info, subs, RegexpInfo::no_match())
        }
    };
    info.simplify(false);
    info
}

fn fold<F>(f: F, subs: &[Hir], zero: RegexpInfo) -> RegexpInfo 
where F: Fn(RegexpInfo, RegexpInfo) -> RegexpInfo {
    if subs.is_empty() {
        return zero;
    }
    if subs.len() == 1 {
        return analyze_hir(&subs[0]);
    }
    let mut info = f(analyze_hir(&subs[0]), analyze_hir(&subs[1]));
    for i in 2..subs.len() {
        info = f(info, analyze_hir(&subs[i]));
    }
    info
}

fn concat_info(x: RegexpInfo, y: RegexpInfo) -> RegexpInfo {
    let mut xy = RegexpInfo::new();
    xy.match_q = x.match_q.clone().and(y.match_q.clone());
    
    let x_exact = x.exact.is_some();
    let y_exact = y.exact.is_some();
    
    if x_exact && y_exact {
        xy.exact = Some(cross_sets(x.exact.as_ref().unwrap(), y.exact.as_ref().unwrap()));
    } else {
        if x_exact {
             xy.prefix = cross_sets(x.exact.as_ref().unwrap(), &y.prefix);
        } else {
             xy.prefix = x.prefix.clone();
             if x.can_empty {
                 xy.prefix = union_sets(xy.prefix, y.prefix.clone());
             }
        }
        
        if y_exact {
            xy.suffix = cross_sets(&x.suffix, y.exact.as_ref().unwrap());
        } else {
            xy.suffix = y.suffix.clone();
            if y.can_empty {
                xy.suffix = union_sets(xy.suffix, x.suffix.clone());
            }
        }
    }
    
    xy.can_empty = x.can_empty && y.can_empty;
    
    // Optimization for boundary trigrams
    if !x_exact && !y_exact && 
       x.suffix.len() <= MAX_SET && y.prefix.len() <= MAX_SET &&
       min_len(&x.suffix) + min_len(&y.prefix) >= 3 {
        xy.match_q = xy.match_q.and_trigrams(cross_sets(&x.suffix, &y.prefix));
    }
    
    xy.simplify(false);
    xy
}

fn alternate_info(mut x: RegexpInfo, mut y: RegexpInfo) -> RegexpInfo {
    let mut xy = RegexpInfo::new();
    let x_exact = x.exact.is_some();
    let y_exact = y.exact.is_some();
    
    if x_exact && y_exact {
        xy.exact = Some(union_sets(x.exact.take().unwrap(), y.exact.take().unwrap()));
    } else if x_exact {
        let xe = x.exact.take().unwrap();
        xy.prefix = union_sets(xe.clone(), y.prefix);
        xy.suffix = union_sets(xe.clone(), y.suffix);
        x.exact = Some(xe); // Restore for add_exact
        x.add_exact();
    } else if y_exact {
        let ye = y.exact.take().unwrap();
        xy.prefix = union_sets(x.prefix, ye.clone());
        xy.suffix = union_sets(x.suffix, ye.clone());
        y.exact = Some(ye);
        y.add_exact();
    } else {
        xy.prefix = union_sets(x.prefix, y.prefix);
        xy.suffix = union_sets(x.suffix, y.suffix);
    }
    
    xy.can_empty = x.can_empty || y.can_empty;
    xy.match_q = x.match_q.or(y.match_q);
    
    xy.simplify(false);
    xy
}