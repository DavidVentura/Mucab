use encoding_rs::EUC_JP;
use glob::glob;
use regex::Regex;
use std::collections::HashMap;
use std::env;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use zeekstd::{EncodeOptions, Encoder, FrameSizePolicy};

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
    let (pos_id_map, entries) = process_csv_files(input_dir);
    println!("Found {} unique pos_ids", pos_id_map.len());
    println!("Processed {} entries", entries.len());

    let matrix_path = format!("{}/matrix.def", input_dir);
    let (matrix_data, matrix_size) =
        load_matrix(&matrix_path, &pos_id_map).expect("Failed to load matrix");

    println!(
        "Matrix: {}x{} = {} entries ({} bytes)",
        matrix_size,
        matrix_size,
        matrix_data.len(),
        matrix_data.len() * 2
    );

    let output_path = format!("{}/mucab.bin", output_dir);
    write_binary(&output_path, &entries, &matrix_data, matrix_size as u16)
        .expect("Failed to write binary");
    println!("Wrote {}", output_path);

    println!("Conversion complete!");
}

struct Entry {
    surface: String,
    pos_id: u16,
    cost: i16,
    reading: String,
}

fn process_csv_files(input_dir: &str) -> (HashMap<String, u16>, Vec<Entry>) {
    let pattern = format!("{}/*.csv", input_dir);
    let han_regex = Regex::new(r"^\p{Han}+").unwrap();

    let mut pos_id_map: HashMap<String, u16> = HashMap::new();
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
                    assert_eq!(
                        left_id_str, right_id_str,
                        "left_id and right_id differ for surface: {}",
                        surface
                    );

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

                    let pos_id_len = pos_id_map.len();
                    let pos_id = *pos_id_map.entry(left_id_str.clone()).or_insert_with(|| {
                        let id = pos_id_len as u16;
                        if id == 65535 {
                            panic!("Too many unique pos_ids! Maximum is 65535.");
                        }
                        id
                    });

                    entries.push(Entry {
                        surface: surface.to_string(),
                        pos_id,
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
            (Some(ac), Some(bc)) => ac.cmp(&bc).then_with(|| a.surface.cmp(&b.surface)),
            _ => a.surface.cmp(&b.surface),
        }
    });

    (pos_id_map, entries)
}

fn load_matrix(
    input_path: &str,
    pos_id_map: &HashMap<String, u16>,
) -> std::io::Result<(Vec<i16>, usize)> {
    let file = File::open(input_path)?;
    let reader = BufReader::new(file);
    let mut lines = reader.lines();

    lines.next(); // skip header

    let matrix_size = pos_id_map.len();
    let mut matrix = vec![0i16; matrix_size * matrix_size];

    for line in lines {
        let line = line?;
        let parts: Vec<&str> = line.split_whitespace().collect();

        if parts.len() >= 3 {
            let prev_id_str = parts[0].to_string();
            let curr_id_str = parts[1].to_string();
            let cost: i16 = parts[2].parse().unwrap_or(0);

            if let (Some(&prev_id), Some(&curr_id)) =
                (pos_id_map.get(&prev_id_str), pos_id_map.get(&curr_id_str))
            {
                // matrix[prev_id][curr_id] = cost
                // Flatten: matrix[prev_id * matrix_size + curr_id] = cost
                let idx = (prev_id as usize) * matrix_size + (curr_id as usize);
                matrix[idx] = cost;
            }
        }
    }

    Ok((matrix, matrix_size))
}

fn write_binary(
    path: &str,
    entries: &[Entry],
    matrix: &[i16],
    matrix_size: u16,
) -> std::io::Result<()> {
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);

    // First, build entry_records with compressed strings
    let mut strings_data = Vec::new(); // Compressed supersequence
    let mut entry_records = Vec::new();

    for entry in entries.iter() {
        let reading_bytes = entry.reading.as_bytes();

        // Find longest suffix of strings_data that matches a prefix of reading
        let mut best_overlap = 0;
        let search_start = strings_data.len().saturating_sub(reading_bytes.len());

        for start in search_start..strings_data.len() {
            let suffix_len = strings_data.len() - start;
            if suffix_len > reading_bytes.len() {
                continue;
            }
            if &strings_data[start..] == &reading_bytes[..suffix_len] {
                best_overlap = suffix_len;
                break;
            }
        }

        let reading_offset = (strings_data.len() - best_overlap) as u32;
        strings_data.extend_from_slice(&reading_bytes[best_overlap..]);
        let reading_len = entry.reading.len() as u8;

        entry_records.push((
            entry.surface.as_bytes().to_vec(), // Store surface bytes directly
            reading_offset,
            reading_len,
            entry.pos_id,
            entry.cost,
        ));
    }

    // Now build index with byte offsets
    let mut index: Vec<(char, u32, u16)> = Vec::new();
    let mut current_char: Option<char> = None;
    let mut current_byte_offset = 0u32;
    let mut current_count = 0u16;
    let mut byte_offset = 0u32;

    for (i, entry) in entries.iter().enumerate() {
        if let Some(first_char) = entry.surface.chars().next() {
            if Some(first_char) != current_char {
                if let Some(ch) = current_char {
                    index.push((ch, current_byte_offset, current_count));
                }
                current_char = Some(first_char);
                current_byte_offset = byte_offset;
                current_count = 1;
            } else {
                current_count += 1;
            }
        }
        // Calculate byte size of this entry for next iteration
        // surf_len(1) + surface + read_off(4) + read_len(1) + pos_id(2) + cost(2) = 1 + surf + 9
        byte_offset += 1 + entry_records[i].0.len() as u32 + 9;
    }
    // Push last group
    if let Some(ch) = current_char {
        index.push((ch, current_byte_offset, current_count));
    }

    eprintln!("Index has {} unique characters", index.len());

    let header_size = 16u32;
    let matrix_byte_size = (matrix.len() * 2) as u32;
    let index_size = 4 + (index.len() as u32 * 10); // num_keys(4) + (char(4) + offset(4) + count(2)) * num_keys

    // Calculate variable entry array size: sum of (1 + surf_len + 9) per entry
    let entry_array_size: u32 = entry_records
        .iter()
        .map(|(surf, _, _, _, _)| 1 + surf.len() as u32 + 9)
        .sum();

    // strings_offset is now relative to start of decompressed stream (after entries)
    let strings_offset = entry_array_size;

    println!("Header: {} bytes", header_size);
    writer.write_all(b"MUCA")?;
    writer.write_all(&1u16.to_le_bytes())?;
    writer.write_all(&matrix_size.to_le_bytes())?;
    writer.write_all(&(entries.len() as u32).to_le_bytes())?;
    writer.write_all(&strings_offset.to_le_bytes())?;

    println!(
        "Matrix: {} bytes ({} entries, {}x{})",
        matrix_byte_size,
        matrix.len(),
        matrix_size,
        matrix_size
    );
    for &cost in matrix {
        writer.write_all(&cost.to_le_bytes())?;
    }

    println!("Index: {} bytes ({} keys)", index_size, index.len());
    writer.write_all(&(index.len() as u32).to_le_bytes())?;
    for (ch, byte_offset, count) in &index {
        writer.write_all(&(*ch as u32).to_le_bytes())?;
        writer.write_all(&byte_offset.to_le_bytes())?;
        writer.write_all(&count.to_le_bytes())?;
    }

    // Create zeekstd encoder for compressed block (entries + strings)
    println!(
        "Compressing entries ({} bytes) + strings ({} bytes)...",
        entry_array_size,
        strings_data.len()
    );

    let opts = EncodeOptions::new()
        .checksum_flag(false)
        .compression_level(9)
        .frame_size_policy(FrameSizePolicy::Uncompressed(1024 * 128));

    let mut encoder = Encoder::with_opts(writer, opts).map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::Other, format!("zeekstd error: {:?}", e))
    })?;

    // Write variable-length entries: surf_len(1) + surface + read_off(4) + read_len(1) + pos_id(2) + cost(2)
    for (surf_bytes, read_off, read_len, pos_id, cost) in &entry_records {
        encoder.write_all(&[surf_bytes.len() as u8])?;
        encoder.write_all(surf_bytes)?;
        encoder.write_all(&read_off.to_le_bytes())?;
        encoder.write_all(&[*read_len])?;
        encoder.write_all(&pos_id.to_le_bytes())?;
        encoder.write_all(&cost.to_le_bytes())?;
    }

    // Write strings immediately after entries in same compressed block
    encoder.write_all(&strings_data)?;

    let compressed_size = encoder.finish().map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::Other, format!("zeekstd error: {:?}", e))
    })?;
    println!(
        "Compressed block: {} bytes (from {} bytes uncompressed, {:.1}% of original)",
        compressed_size,
        entry_array_size + strings_data.len() as u32,
        100.0 * compressed_size as f64 / (entry_array_size + strings_data.len() as u32) as f64
    );

    Ok(())
}
