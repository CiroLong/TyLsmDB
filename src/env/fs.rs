use std::fmt::Debug;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::error::{Error, Result};

const CURRENT_FILE: &str = "CURRENT";
const CURRENT_TMP_FILE: &str = "CURRENT.tmp";

#[derive(Debug, Clone, Copy)]
pub struct WritableFileOptions {
    pub create: bool,
    pub truncate: bool,
    pub append: bool,
    pub read: bool,
}

impl WritableFileOptions {
    pub fn create() -> Self {
        Self {
            create: true,
            truncate: true,
            append: false,
            read: false,
        }
    }

    pub fn append() -> Self {
        Self {
            create: true,
            truncate: false,
            append: true,
            read: true,
        }
    }
}

pub trait WritableFile: Debug + Send {
    fn write_all(&mut self, bytes: &[u8]) -> Result<()>;
    fn sync_all(&mut self) -> Result<()>;
}

pub trait ReadableFile: Debug + Send {
    fn read(&mut self, dst: &mut [u8]) -> Result<usize>;
    fn read_exact(&mut self, dst: &mut [u8]) -> Result<()>;
    fn seek(&mut self, pos: SeekFrom) -> Result<u64>;
    fn len(&self) -> Result<u64>;

    fn is_empty(&self) -> Result<bool> {
        Ok(self.len()? == 0)
    }
}

pub trait Env: Debug + Send + Sync {
    fn create_dir_all(&self, path: &Path) -> Result<()>;
    fn exists(&self, path: &Path) -> bool;
    fn open_writable(
        &self,
        path: &Path,
        options: WritableFileOptions,
    ) -> Result<Box<dyn WritableFile>>;
    fn open_readable(&self, path: &Path) -> Result<Box<dyn ReadableFile>>;
    fn read_to_string(&self, path: &Path) -> Result<String>;
    fn rename(&self, from: &Path, to: &Path) -> Result<()>;
    fn metadata_len(&self, path: &Path) -> Result<u64>;
    fn remove_file(&self, path: &Path) -> Result<()>;
    fn sync_directory(&self, path: &Path) -> Result<()>;
}

#[derive(Debug, Default)]
pub struct FsEnv;

impl Env for FsEnv {
    fn create_dir_all(&self, path: &Path) -> Result<()> {
        std::fs::create_dir_all(path)?;
        Ok(())
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn open_writable(
        &self,
        path: &Path,
        options: WritableFileOptions,
    ) -> Result<Box<dyn WritableFile>> {
        let file = OpenOptions::new()
            .create(options.create)
            .truncate(options.truncate)
            .append(options.append)
            .write(true)
            .read(options.read)
            .open(path)?;
        Ok(Box::new(FsWritableFile { file }))
    }

    fn open_readable(&self, path: &Path) -> Result<Box<dyn ReadableFile>> {
        Ok(Box::new(FsReadableFile {
            file: File::open(path)?,
        }))
    }

    fn read_to_string(&self, path: &Path) -> Result<String> {
        Ok(std::fs::read_to_string(path)?)
    }

    fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        std::fs::rename(from, to)?;
        Ok(())
    }

    fn metadata_len(&self, path: &Path) -> Result<u64> {
        Ok(std::fs::metadata(path)?.len())
    }

    fn remove_file(&self, path: &Path) -> Result<()> {
        std::fs::remove_file(path)?;
        Ok(())
    }

    fn sync_directory(&self, path: &Path) -> Result<()> {
        sync_directory(path)
    }
}

#[derive(Debug)]
struct FsWritableFile {
    file: File,
}

impl WritableFile for FsWritableFile {
    fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
        self.file.write_all(bytes)?;
        Ok(())
    }

    fn sync_all(&mut self) -> Result<()> {
        self.file.sync_all()?;
        Ok(())
    }
}

#[derive(Debug)]
struct FsReadableFile {
    file: File,
}

impl ReadableFile for FsReadableFile {
    fn read(&mut self, dst: &mut [u8]) -> Result<usize> {
        Ok(self.file.read(dst)?)
    }

    fn read_exact(&mut self, dst: &mut [u8]) -> Result<()> {
        self.file.read_exact(dst)?;
        Ok(())
    }

    fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        Ok(self.file.seek(pos)?)
    }

    fn len(&self) -> Result<u64> {
        Ok(self.file.metadata()?.len())
    }
}

pub fn set_current(db_path: &Path, manifest_name: &str) -> Result<()> {
    let env = FsEnv;
    set_current_with_env(&env, db_path, manifest_name)
}

pub fn set_current_with_env(env: &dyn Env, db_path: &Path, manifest_name: &str) -> Result<()> {
    let tmp_path = db_path.join(CURRENT_TMP_FILE);
    let current_path = db_path.join(CURRENT_FILE);

    {
        let mut file = env.open_writable(&tmp_path, WritableFileOptions::create())?;
        file.write_all(manifest_name.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
    }

    env.rename(&tmp_path, &current_path)?;
    env.sync_directory(db_path)?;
    Ok(())
}

pub fn read_current(db_path: &Path) -> Result<String> {
    let env = FsEnv;
    read_current_with_env(&env, db_path)
}

pub fn read_current_with_env(env: &dyn Env, db_path: &Path) -> Result<String> {
    let current = env.read_to_string(&db_path.join(CURRENT_FILE))?;
    let manifest_name = current.trim_end_matches(['\r', '\n']);
    if manifest_name.is_empty() {
        return Err(Error::Corruption("CURRENT is empty".to_string()));
    }
    Ok(manifest_name.to_string())
}

fn sync_directory(path: &Path) -> Result<()> {
    if let Ok(dir) = File::open(path) {
        dir.sync_all()?;
    }
    Ok(())
}
