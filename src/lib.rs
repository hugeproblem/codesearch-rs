pub mod sparse_set;
pub mod index;

use std::path::Path;
use std::env;

/// Find the index file using the following priority:
/// 1. Look for .csearchindex in current directory
/// 2. Walk up directory tree looking for .csearchindex
/// 3. Check CSEARCHINDEX environment variable
/// 4. Check HOME/.csearchindex
/// 5. For cindex: create in current directory, for csearch: return error
pub fn find_index_file(create_if_missing: bool) -> anyhow::Result<String> {
    // 1. Check current directory
    let current_dir = env::current_dir()?;
    let mut dir = current_dir.as_path();
    
    // 2. Walk up directory tree
    loop {
        let index_path = dir.join(".csearchindex");
        if index_path.exists() {
            return Ok(index_path.to_string_lossy().to_string());
        }
        
        match dir.parent() {
            Some(parent) => dir = parent,
            None => break,
        }
    }
    
    // 3. Check CSEARCHINDEX environment variable
    if let Ok(env_path) = env::var("CSEARCHINDEX") {
        if Path::new(&env_path).exists() {
            return Ok(env_path);
        }
    }
    
    // 4. Check HOME/.csearchindex
    if let Ok(home) = env::var("HOME") {
        let home_index = format!("{}/.csearchindex", home);
        if Path::new(&home_index).exists() {
            return Ok(home_index);
        }
    }
    
    // 5. If nothing found
    if create_if_missing {
        // For cindex: create in current directory
        let current_index = current_dir.join(".csearchindex");
        Ok(current_index.to_string_lossy().to_string())
    } else {
        // For csearch: try environment variable or HOME as fallback
        if let Ok(env_path) = env::var("CSEARCHINDEX") {
            return Ok(env_path);
        }
        
        if let Ok(home) = env::var("HOME") {
            return Ok(format!("{}/.csearchindex", home));
        }
        
        anyhow::bail!("No index file found. Run cindex to create one.")
    }
}
