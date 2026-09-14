//! Verification for the P4K reader: pull the three config files BindSight
//! needs out of a real `Data.p4k` (CryXmlB decoded) into a folder, to diff
//! against another extractor's output. Run with:
//!
//!     cargo run --example p4k_extract -- <Data.p4k> <out dir>

use std::path::Path;

use bindsight_lib::{cryxml, p4k};

const FILES: [&str; 3] = [
    "Data/Libs/Config/defaultProfile.xml",
    "Data/Libs/Config/keybinding_localization.xml",
    "Data/Localization/english/global.ini",
];

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let p4k_path = args.next().ok_or("usage: p4k_extract <Data.p4k> <out dir>")?;
    let out = args.next().ok_or("usage: p4k_extract <Data.p4k> <out dir>")?;

    let start = std::time::Instant::now();
    let mut archive = p4k::Archive::open(Path::new(&p4k_path))?;
    println!("{} entries in {:.2?}", archive.entries().len(), start.elapsed());

    for name in FILES {
        let entry = archive.entry(name).ok_or(format!("not in the archive: {name}"))?.clone();
        println!(
            "{name}: method={} encrypted={} compressed={} uncompressed={} offset={}",
            entry.compression_method, entry.encrypted, entry.compressed_size, entry.uncompressed_size, entry.offset
        );
        let start = std::time::Instant::now();
        let mut bytes = archive.read(&entry)?;
        if cryxml::is_cryxmlb(&bytes) {
            bytes = cryxml::to_xml(&bytes)?.into_bytes();
            println!("  CryXmlB decoded");
        }
        println!("  {} bytes in {:.2?}", bytes.len(), start.elapsed());
        let target = Path::new(&out).join(name);
        std::fs::create_dir_all(target.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&target, bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}
