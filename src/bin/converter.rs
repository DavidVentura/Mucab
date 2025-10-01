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
    let (left_id_map, right_id_map, entries) = process_csv_files(input_dir);
    println!(
        "Found {} unique left_ids, {} unique right_ids",
        left_id_map.len(),
        right_id_map.len()
    );
    println!("Processed {} entries", entries.len());

    let matrix_path = format!("{}/matrix.def", input_dir);
    let (matrix_data, max_left, max_right) =
        load_matrix(&matrix_path, &left_id_map, &right_id_map).expect("Failed to load matrix");

    println!(
        "Matrix before compaction: would be {}x{} = {} entries ({} bytes)",
        left_id_map.len(),
        right_id_map.len(),
        left_id_map.len() * right_id_map.len(),
        left_id_map.len() * right_id_map.len() * 2
    );
    println!(
        "Matrix after compaction: {}x{} = {} entries ({} bytes)",
        max_left,
        max_right,
        matrix_data.len(),
        matrix_data.len() * 2
    );

    let output_path = format!("{}/mucab.bin", output_dir);
    write_binary(
        &output_path,
        &entries,
        &matrix_data,
        max_left as u16,
        max_right as u16,
    )
    .expect("Failed to write binary");
    println!("Wrote {}", output_path);

    println!("Conversion complete!");
}

struct Entry {
    surface: String,
    left_id: u16,
    right_id: u16,
    cost: i16,
    reading: String,
}

fn process_csv_files(input_dir: &str) -> (HashMap<String, u16>, HashMap<String, u16>, Vec<Entry>) {
    let pattern = format!("{}/*.csv", input_dir);
    let han_regex = Regex::new(r"^\p{Han}+").unwrap();

    let mut left_id_map: HashMap<String, u16> = HashMap::new();
    let mut right_id_map: HashMap<String, u16> = HashMap::new();
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

                    if surface.len() > 255 {
                        eprintln!("Warning: surface too long ({}), skipping", surface.len());
                        continue;
                    }

                    let left_id_str = parts[1].to_string();
                    let right_id_str = parts[2].to_string();
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

                    // Assign sequential IDs
                    let left_id_len = left_id_map.len();
                    let left_id = *left_id_map.entry(left_id_str.clone()).or_insert_with(|| {
                        let id = left_id_len as u16;
                        if id == 65535 {
                            panic!("Too many unique left_ids! Maximum is 65535.");
                        }
                        id
                    });

                    let right_id_len = right_id_map.len();
                    let right_id = *right_id_map.entry(right_id_str.clone()).or_insert_with(|| {
                        let id = right_id_len as u16;
                        if id == 65535 {
                            panic!("Too many unique right_ids! Maximum is 65535.");
                        }
                        id
                    });

                    entries.push(Entry {
                        surface: surface.to_string(),
                        left_id,
                        right_id,
                        cost,
                        reading,
                    });
                }
            }
            Err(e) => eprintln!("Error reading glob entry: {}", e),
        }
    }

    // Sort by first character, then by surface
    entries.sort_by(|a, b| {
        let a_first = a.surface.chars().next();
        let b_first = b.surface.chars().next();
        match (a_first, b_first) {
            (Some(ac), Some(bc)) => {
                ac.cmp(&bc).then_with(|| a.surface.cmp(&b.surface))
            }
            _ => a.surface.cmp(&b.surface),
        }
    });

    let kita_count = entries
        .iter()
        .filter(|e| e.surface.starts_with('北'))
        .count();
    eprintln!("Debug converter: {} entries start with '北'", kita_count);

    (left_id_map, right_id_map, entries)
}

fn load_matrix(
    input_path: &str,
    left_id_map: &HashMap<String, u16>,
    right_id_map: &HashMap<String, u16>,
) -> std::io::Result<(Vec<i16>, usize, usize)> {
    let file = File::open(input_path)?;
    let reader = BufReader::new(file);
    let mut lines = reader.lines();

    lines.next(); // skip header

    let max_left = left_id_map.len();
    let max_right = right_id_map.len();
    let mut matrix = vec![0i16; max_left * max_right];

    for line in lines {
        let line = line?;
        let parts: Vec<&str> = line.split_whitespace().collect();

        if parts.len() >= 3 {
            let left_id_str = parts[0].to_string();
            let right_id_str = parts[1].to_string();
            let cost: i16 = parts[2].parse().unwrap_or(0);

            if let (Some(&left_id), Some(&right_id)) = (
                left_id_map.get(&left_id_str),
                right_id_map.get(&right_id_str),
            ) {
                // matrix[prev.right_id][curr.left_id] = cost
                // Flatten: matrix[right_id * max_left + left_id] = cost
                let idx = (right_id as usize) * max_left + (left_id as usize);
                matrix[idx] = cost;
            }
        }
    }

    Ok((matrix, max_left, max_right))
}

fn write_binary(
    path: &str,
    entries: &[Entry],
    matrix: &[i16],
    max_left: u16,
    max_right: u16,
) -> std::io::Result<()> {
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);

    // Build index as (char, count) since entries are sorted by first char
    let mut index: Vec<(char, u16)> = Vec::new();
    let mut current_char: Option<char> = None;
    let mut current_count = 0u16;

    for entry in entries.iter() {
        if let Some(first_char) = entry.surface.chars().next() {
            if Some(first_char) != current_char {
                if let Some(ch) = current_char {
                    index.push((ch, current_count));
                }
                current_char = Some(first_char);
                current_count = 1;
            } else {
                current_count += 1;
            }
        }
    }
    // Push last group
    if let Some(ch) = current_char {
        index.push((ch, current_count));
    }

    eprintln!("Index has {} unique characters", index.len());

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
            entry.left_id,
            entry.right_id,
            entry.cost,
        ));
    }

    let header_size = 22u32;
    let matrix_size = (matrix.len() * 2) as u32;
    let entry_array_size = (entry_records.len() * 16) as u32;

    let index_offset = header_size + matrix_size + entry_array_size;
    let index_size = 4 + (index.len() as u32 * 6);  // num_keys(4) + (char(4) + count(2)) * num_keys
    let strings_offset = index_offset + index_size;

    println!("Header: {} bytes", header_size);
    writer.write_all(b"MUCA")?;
    writer.write_all(&1u16.to_le_bytes())?;
    writer.write_all(&max_left.to_le_bytes())?;
    writer.write_all(&max_right.to_le_bytes())?;
    writer.write_all(&(entries.len() as u32).to_le_bytes())?;
    writer.write_all(&index_offset.to_le_bytes())?;
    writer.write_all(&strings_offset.to_le_bytes())?;

    println!(
        "Matrix: {} bytes ({} entries, {}x{})",
        matrix_size,
        matrix.len(),
        max_left,
        max_right
    );
    for &cost in matrix {
        writer.write_all(&cost.to_le_bytes())?;
    }

    println!(
        "Entries: {} bytes ({} entries)",
        entry_array_size,
        entry_records.len()
    );

    let file2 = File::create("strings")?;
    let mut w2 = BufWriter::new(file2);
    for (surf_off, surf_len, read_off, read_len, left_id, right_id, cost) in &entry_records {
        writer.write_all(&surf_off.to_le_bytes())?;
        writer.write_all(&[*surf_len])?;
        writer.write_all(&read_off.to_le_bytes())?;
        writer.write_all(&[*read_len])?;
        writer.write_all(&left_id.to_le_bytes())?;
        writer.write_all(&right_id.to_le_bytes())?;
        writer.write_all(&cost.to_le_bytes())?;
    }

    println!("Index: {} bytes ({} keys)", index_size, index.len());
    writer.write_all(&(index.len() as u32).to_le_bytes())?;
    for (ch, count) in &index {
        writer.write_all(&(*ch as u32).to_le_bytes())?;
        writer.write_all(&count.to_le_bytes())?;
    }

    println!("Strings: {} bytes", strings_data.len());
    writer.write_all(&strings_data)?;
    w2.write_all(&strings_data)?;

    Ok(())
}
