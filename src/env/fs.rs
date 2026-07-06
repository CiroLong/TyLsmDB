use std::fs::File;
use std::io::Write;
use std::path::Path;

use crate::error::{Error, Result};

const CURRENT_FILE: &str = "CURRENT";
const CURRENT_TMP_FILE: &str = "CURRENT.tmp";

pub fn set_current(db_path: &Path, manifest_name: &str) -> Result<()> {
    let tmp_path = db_path.join(CURRENT_TMP_FILE);
    let current_path = db_path.join(CURRENT_FILE);

    {
        let mut file = File::create(&tmp_path)?;
        file.write_all(manifest_name.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
    }

    std::fs::rename(&tmp_path, current_path)?;
    sync_directory(db_path);
    Ok(())
}

pub fn read_current(db_path: &Path) -> Result<String> {
    let current = std::fs::read_to_string(db_path.join(CURRENT_FILE))?;
    let manifest_name = current.trim_end_matches(['\r', '\n']);
    if manifest_name.is_empty() {
        return Err(Error::Corruption("CURRENT is empty".to_string()));
    }
    Ok(manifest_name.to_string())
}

fn sync_directory(path: &Path) {
    if let Ok(dir) = File::open(path) {
        let _ = dir.sync_all();
    }
}
