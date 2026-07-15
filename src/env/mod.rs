pub mod file;
pub mod fs;

pub use fs::{
    Env, FsEnv, ReadableFile, WritableFile, WritableFileOptions, read_current,
    read_current_with_env, set_current, set_current_with_env,
};

#[cfg(test)]
mod tests;
