pub fn wal_file_name(number: u64) -> String {
    format!("{number:06}.wal")
}

pub fn table_file_name(number: u64) -> String {
    format!("{number:06}.sst")
}

pub fn manifest_file_name(number: u64) -> String {
    format!("MANIFEST-{number:06}")
}

pub fn parse_manifest_number(name: &str) -> Option<u64> {
    name.strip_prefix("MANIFEST-")?.parse().ok()
}
