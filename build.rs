use std::{fs::File, io::{BufRead, BufReader}, collections::HashSet};
use std::fs::write;
use std::env;
use std::path::{PathBuf, Path};

fn main() {
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/neovide.ico");
        res.compile().expect("Could not attach exe icon");
    }

    let parsed = parse_emoji_presentation("src/renderer/fonts/emoji-data.txt");
    let out_file = Path::new(&env::var("OUT_DIR").unwrap()).join("emoji.rs");
    emit_table(&parsed, out_file);
}

fn parse_emoji_presentation(path: &str) -> HashSet<u32> {
    let file = File::open(path).unwrap();
    let reader = BufReader::new(file);
    let mut set = HashSet::new();

    for line in reader.lines().flatten() {
        if let Some((range, prop)) = line.split_once(';') {
            if prop.trim() == "Emoji_Presentation" {
                let range = range.trim();
                if let Some((start, end)) = range.split_once("..") {
                    let start = u32::from_str_radix(start, 16).unwrap();
                    let end = u32::from_str_radix(end, 16).unwrap();
                    for cp in start..=end {
                        set.insert(cp);
                    }
                } else {
                    let cp = u32::from_str_radix(range, 16).unwrap();
                    set.insert(cp);
                }
            }
        }
    }

    set
}

fn emit_table(set: &HashSet<u32>, path: PathBuf) {
    let mut out = String::from("pub fn is_emoji_presentation(cp: u32) -> bool {\n    match cp {\n");
    for cp in set.iter().copied().collect::<Vec<_>>() {
        out += &format!("        0x{:X} => true,\n", cp);
    }
    out += "        _ => false,\n    }\n}\n";

    write(path, out).unwrap();
}
