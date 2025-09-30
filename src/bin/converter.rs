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

    let matrix_path = format!("{}/matrix.def", input_dir);
    let matrix_data = load_matrix(&matrix_path, &pair_map).expect("Failed to load matrix");

    let output_path = format!("{}/mucab.bin", output_dir);
    write_binary(&output_path, &entries, &matrix_data).expect("Failed to write binary");
    println!("Wrote {}", output_path);

    println!("Conversion complete!");
}

struct Entry {
    surface: String,
    pair_id: u16,
    cost: i16,
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

                    if surface.len() > 255 {
                        eprintln!("Warning: surface too long ({}), skipping", surface.len());
                        continue;
                    }

                    let left_id = parts[1].to_string();
                    let right_id = parts[2].to_string();
                    let cost: i32 = parts[3].parse().unwrap_or(0);
                    if cost < i16::MIN as i32 || cost > i16::MAX as i32 {
                        eprintln!("Warning: cost out of range ({}), skipping", cost);
                        continue;
                    }
                    let cost = cost as i16;

                    let reading = parts[12].to_string();
                    if reading.len() > 255 {
                        eprintln!("Warning: reading too long ({}), skipping", reading.len());
                        continue;
                    }

                    let pair = (left_id.clone(), right_id.clone());
                    let pair_id = *pair_map.entry(pair).or_insert_with(|| {
                        if next_pair_id == 65535 {
                            panic!("Too many unique pair_ids! Maximum is 65535.");
                        }
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

fn load_matrix(
    input_path: &str,
    pair_map: &HashMap<(String, String), u16>,
) -> std::io::Result<Vec<i16>> {
    let file = File::open(input_path)?;
    let reader = BufReader::new(file);
    let mut lines = reader.lines();

    lines.next();

    let mut matrix = vec![0i16; 1024]; // FIXME hardcoded bound

    for line in lines {
        let line = line?;
        let parts: Vec<&str> = line.split_whitespace().collect();

        if parts.len() >= 3 {
            let left_id = parts[0].to_string();
            let right_id = parts[1].to_string();
            let cost: i16 = parts[2].parse().unwrap_or(0);

            if let Some(&pair_id) = pair_map.get(&(left_id, right_id)) {
                matrix[pair_id as usize] = cost;
            }
        }
    }

    Ok(matrix)
}

fn write_binary(path: &str, entries: &[Entry], matrix: &[i16]) -> std::io::Result<()> {
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);

    let mut index: HashMap<char, Vec<u32>> = HashMap::new();
    for (idx, entry) in entries.iter().enumerate() {
        if let Some(first_char) = entry.surface.chars().next() {
            index.entry(first_char).or_default().push(idx as u32);
        }
    }

    let mut strings_data = Vec::new();
    let mut entry_records = Vec::new();

    for entry in entries {
        let surface_offset = strings_data.len() as u32;
        strings_data.extend_from_slice(entry.surface.as_bytes());
        let surface_len = entry.surface.len() as u8;

        let reading_offset = strings_data.len() as u32;
        strings_data.extend_from_slice(entry.reading.as_bytes());
        let reading_len = entry.reading.len() as u8;

        entry_records.push((
            surface_offset,
            surface_len,
            reading_offset,
            reading_len,
            entry.pair_id,
            entry.cost,
        ));
    }

    let header_size = 20u32;
    let matrix_size = (matrix.len() * 4) as u32;
    let entry_array_size = (entry_records.len() * 16) as u32;

    let index_offset = header_size + matrix_size + entry_array_size;
    let strings_offset = index_offset
        + 4
        + (index.len() as u32 * 8)
        + index.values().map(|v| v.len() as u32 * 4).sum::<u32>();

    println!("Header: {} bytes", header_size);
    writer.write_all(b"MUCA")?;
    writer.write_all(&1u16.to_le_bytes())?;
    writer.write_all(&(matrix.len() as u16).to_le_bytes())?;
    writer.write_all(&(entries.len() as u32).to_le_bytes())?;
    writer.write_all(&index_offset.to_le_bytes())?;
    writer.write_all(&strings_offset.to_le_bytes())?;

    println!("Matrix: {} bytes ({} entries)", matrix_size, matrix.len());
    for (idx, &cost) in matrix.iter().enumerate() {
        writer.write_all(&(idx as u16).to_le_bytes())?;
        writer.write_all(&cost.to_le_bytes())?;
    }

    println!(
        "Entries: {} bytes ({} entries)",
        entry_array_size,
        entry_records.len()
    );
    for (surf_off, surf_len, read_off, read_len, pair_id, cost) in &entry_records {
        writer.write_all(&surf_off.to_le_bytes())?;
        writer.write_all(&[*surf_len])?;
        writer.write_all(&read_off.to_le_bytes())?;
        writer.write_all(&[*read_len])?;
        writer.write_all(&pair_id.to_le_bytes())?;
        writer.write_all(&cost.to_le_bytes())?;
        writer.write_all(&[0u8, 0u8])?; // padding to 16 bytes
    }

    let index_size = 4 + index.iter().map(|(_, v)| 8 + v.len() * 4).sum::<usize>();
    println!("Index: {} bytes ({} keys)", index_size, index.len());
    writer.write_all(&(index.len() as u32).to_le_bytes())?;
    for (ch, entry_ids) in &index {
        writer.write_all(&(*ch as u32).to_le_bytes())?;
        writer.write_all(&(entry_ids.len() as u16).to_le_bytes())?;
        writer.write_all(&[0u8; 2])?; // padding
        for &entry_id in entry_ids {
            writer.write_all(&entry_id.to_le_bytes())?;
        }
    }

    println!("Strings: {} bytes", strings_data.len());
    writer.write_all(&strings_data)?;

    Ok(())
}
