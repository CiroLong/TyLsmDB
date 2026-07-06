use crate::error::{Error, Result};

pub fn require_non_empty(input: &[u8], context: &'static str) -> Result<()> {
    if input.is_empty() {
        return Err(Error::InvalidArgument(format!(
            "{context} must not be empty"
        )));
    }
    Ok(())
}
