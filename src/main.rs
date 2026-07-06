use tylsmdb::{DB, Options};

fn main() -> tylsmdb::Result<()> {
    let db = DB::open("target/tylsmdb-example", Options::default())?;
    println!("opened TYLSMDB at {}", db.path().display());
    Ok(())
}
