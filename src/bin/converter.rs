use encoding_rs::EUC_JP;
use glob::glob;
use regex::Regex;
use std::collections::HashMap;
use std::env;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: {} <input_dir> <output_dir>", args[0]);
        std::process::exit(1);
    }

    let input_dir = &args[1];
    let output_dir = &args[2];

    std::fs::create_dir_all(output_dir).expect("Failed to create output directory");

    println!("Processing CSV files from {}...", input_dir);
    let (pair_map, entries) = process_csv_files(input_dir);
    println!("Found {} unique (left_id, right_id) pairs", pair_map.len());
    println!("Processed {} entries", entries.len());

    let minidef_path = format!("{}/minidef.csv", output_dir);
    write_minidef(&minidef_path, &entries).expect("Failed to write minidef.csv");
    println!("Wrote {}", minidef_path);

    let matrix_path = format!("{}/matrix.def", input_dir);
    let smatrix_path = format!("{}/smatrix.def", output_dir);
    process_matrix(&matrix_path, &smatrix_path, &pair_map).expect("Failed to process matrix");
    println!("Wrote {}", smatrix_path);

    println!("Conversion complete!");
}

struct Entry {
    surface: String,
    pair_id: u16,
    cost: i32,
    reading: String,
}

fn process_csv_files(input_dir: &str) -> (HashMap<(String, String), u16>, Vec<Entry>) {
    let pattern = format!("{}/*.csv", input_dir);
    let han_regex = Regex::new(r"^\p{Han}+").unwrap();

    let mut pair_map: HashMap<(String, String), u16> = HashMap::new();
    let mut next_pair_id = 1u16;
    let mut entries = Vec::new();

    for entry in glob(&pattern).expect("Failed to read glob pattern") {
        match entry {
            Ok(path) => {
                println!("Processing {:?}...", path);
                let file = File::open(&path).expect("Failed to open file");
                let mut reader = BufReader::new(file);

                let mut buffer = Vec::new();
                reader
                    .read_to_end(&mut buffer)
                    .expect("Failed to read file");

                let (decoded, _, had_errors) = EUC_JP.decode(&buffer);
                if had_errors {
                    eprintln!("Warning: encoding errors in {:?}", path);
                }

                for line in decoded.lines() {
                    let parts: Vec<&str> = line.split(',').collect();
                    if parts.len() < 13 {
                        continue;
                    }

                    let surface = parts[0];
                    if !han_regex.is_match(surface) {
                        continue;
                    }

                    let left_id = parts[1].to_string();
                    let right_id = parts[2].to_string();
                    let cost: i32 = parts[3].parse().unwrap_or(0);
                    let reading = parts[12].to_string();

                    let pair = (left_id.clone(), right_id.clone());
                    let pair_id = *pair_map.entry(pair).or_insert_with(|| {
                        let id = next_pair_id;
                        next_pair_id += 1;
                        id
                    });

                    entries.push(Entry {
                        surface: surface.to_string(),
                        pair_id,
                        cost,
                        reading,
                    });
                }
            }
            Err(e) => eprintln!("Error reading glob entry: {}", e),
        }
    }

    entries.sort_by(|a, b| a.surface.cmp(&b.surface));
    (pair_map, entries)
}

fn write_minidef(path: &str, entries: &[Entry]) -> std::io::Result<()> {
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);

    for entry in entries {
        writeln!(
            writer,
            "{},{},{},{}",
            entry.surface, entry.pair_id, entry.cost, entry.reading
        )?;
    }

    Ok(())
}

fn process_matrix(
    input_path: &str,
    output_path: &str,
    pair_map: &HashMap<(String, String), u16>,
) -> std::io::Result<()> {
    let file = File::open(input_path)?;
    let reader = BufReader::new(file);
    let mut lines = reader.lines();

    lines.next();

    let output_file = File::create(output_path)?;
    let mut writer = BufWriter::new(output_file);

    let mut written_count = 0;

    for line in lines {
        let line = line?;
        let parts: Vec<&str> = line.split_whitespace().collect();

        if parts.len() >= 3 {
            let left_id = parts[0].to_string();
            let right_id = parts[1].to_string();
            let cost: i16 = parts[2].parse().unwrap_or(0);

            if let Some(&pair_id) = pair_map.get(&(left_id, right_id)) {
                let packed = [
                    (pair_id & 0xFF) as u8,
                    ((pair_id >> 8) & 0xFF) as u8,
                    (cost & 0xFF) as u8,
                    ((cost >> 8) & 0xFF) as u8,
                ];
                writer.write_all(&packed)?;
                written_count += 1;
            }
        }
    }

    println!("Wrote {} connection costs", written_count);

    Ok(())
}
