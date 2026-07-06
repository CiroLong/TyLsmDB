use std::ops::Bound;

use crate::bytes::Bytes;

pub type ReadRange = (Bound<Bytes>, Bound<Bytes>);
