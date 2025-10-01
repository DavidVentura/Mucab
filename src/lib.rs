use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};

#[derive(Debug, Clone)]
pub struct DictEntry {
    pub surface: String,
    pub left_id: u16,
    pub right_id: u16,
    pub word_cost: i16,
    pub reading: String,
}

pub struct Dictionary {
    file: BufReader<File>,
    entry_offset: u64,
    strings_offset: u64,
    pub num_entries: usize,
    index: HashMap<char, (usize, usize)>,  // char -> (start_offset, count)
    matrix: Vec<i16>,
    max_left: usize,
}

#[derive(Debug, Clone)]
struct LatticeNode {
    start_pos: usize,
    end_pos: usize,
    entry_idx: usize,
    cost: i32,
    prev_node: Option<usize>,
}

impl Dictionary {
    fn get_matrix_cost(&self, prev_right_id: u16, curr_left_id: u16) -> i16 {
        let idx = (prev_right_id as usize) * self.max_left + (curr_left_id as usize);
        self.matrix.get(idx).copied().unwrap_or(0)
    }

    fn read_entry(&mut self, entry_idx: usize) -> DictEntry {
        // Read entry record (16 bytes)
        let pos = self.entry_offset + (entry_idx * 16) as u64;
        self.file.seek(SeekFrom::Start(pos)).unwrap();

        let mut entry_buf = [0u8; 16];
        self.file.read_exact(&mut entry_buf).unwrap();

        let surf_off =
            u32::from_le_bytes([entry_buf[0], entry_buf[1], entry_buf[2], entry_buf[3]]) as u64;
        let surf_len = entry_buf[4] as usize;
        let read_off =
            u32::from_le_bytes([entry_buf[5], entry_buf[6], entry_buf[7], entry_buf[8]]) as u64;
        let read_len = entry_buf[9] as usize;
        let left_id = u16::from_le_bytes([entry_buf[10], entry_buf[11]]);
        let right_id = u16::from_le_bytes([entry_buf[12], entry_buf[13]]);
        let cost = i16::from_le_bytes([entry_buf[14], entry_buf[15]]);

        // Read both strings in one operation
        let min_off = surf_off.min(read_off);
        let max_end = (surf_off + surf_len as u64).max(read_off + read_len as u64);
        let total_len = (max_end - min_off) as usize;

        self.file.seek(SeekFrom::Start(self.strings_offset + min_off)).unwrap();
        let mut strings_buf = vec![0u8; total_len];
        self.file.read_exact(&mut strings_buf).unwrap();

        let surf_start = (surf_off - min_off) as usize;
        let read_start = (read_off - min_off) as usize;

        let surface_bytes = &strings_buf[surf_start..surf_start + surf_len];
        let reading_bytes = &strings_buf[read_start..read_start + read_len];

        DictEntry {
            surface: String::from_utf8(surface_bytes.into()).unwrap(),
            left_id,
            right_id,
            word_cost: cost,
            reading: String::from_utf8(reading_bytes.into()).unwrap(),
        }
    }

    pub fn load(path: &str) -> std::io::Result<Self> {
        let mut _file = File::open(path)?;
        let mut file = BufReader::new(_file);

        // Read header (22 bytes)
        let mut header = [0u8; 22];
        file.read_exact(&mut header)?;

        if &header[0..4] != b"MUCA" {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid magic number",
            ));
        }

        let _version = u16::from_le_bytes([header[4], header[5]]);
        let max_left = u16::from_le_bytes([header[6], header[7]]) as usize;
        let max_right = u16::from_le_bytes([header[8], header[9]]) as usize;
        let num_entries =
            u32::from_le_bytes([header[10], header[11], header[12], header[13]]) as usize;
        let index_offset =
            u32::from_le_bytes([header[14], header[15], header[16], header[17]]) as u64;
        let strings_offset =
            u32::from_le_bytes([header[18], header[19], header[20], header[21]]) as u64;

        // Read matrix
        let matrix_size = max_left * max_right;
        let mut matrix_bytes = vec![0u8; matrix_size * 2];
        file.read_exact(&mut matrix_bytes)?;

        let mut matrix = vec![0i16; matrix_size];
        for i in 0..matrix_size {
            matrix[i] = i16::from_le_bytes([matrix_bytes[i * 2], matrix_bytes[i * 2 + 1]]);
        }

        let entry_offset = file.stream_position()?;

        // Skip to index
        file.seek(SeekFrom::Start(index_offset))?;

        // Read index (now just char + count)
        let mut index_count_buf = [0u8; 4];
        file.read_exact(&mut index_count_buf)?;
        let num_index_keys = u32::from_le_bytes(index_count_buf) as usize;

        let mut index: HashMap<char, (usize, usize)> = HashMap::new();
        let mut cumulative_offset = 0usize;

        for _ in 0..num_index_keys {
            let mut char_buf = [0u8; 4];
            file.read_exact(&mut char_buf)?;
            let ch = char::from_u32(u32::from_le_bytes(char_buf)).unwrap();

            let mut count_buf = [0u8; 2];
            file.read_exact(&mut count_buf)?;
            let count = u16::from_le_bytes(count_buf) as usize;

            index.insert(ch, (cumulative_offset, count));
            cumulative_offset += count;
        }

        eprintln!(
            "Debug: loaded {} index keys, matrix {}x{} = {} entries",
            index.len(),
            max_left,
            max_right,
            matrix.len()
        );

        Ok(Dictionary {
            file,
            entry_offset,
            strings_offset,
            num_entries,
            index,
            matrix,
            max_left,
        })
    }

    fn lookup(&mut self, text: &str, start: usize) -> Vec<usize> {
        let chars: Vec<char> = text.chars().collect();
        if start >= chars.len() {
            return Vec::with_capacity(1024);
        }

        let first_char = chars[start];
        let mut matches = Vec::with_capacity(1024);

        if let Some(&(start_offset, count)) = self.index.get(&first_char) {
            // Scan entries from start_offset to start_offset + count
            for i in 0..count {
                let entry_idx = start_offset + i;
                let entry = self.read_entry(entry_idx);
                let entry_chars: Vec<char> = entry.surface.chars().collect();

                if start + entry_chars.len() <= chars.len() {
                    let matches_surface = entry_chars
                        .iter()
                        .enumerate()
                        .all(|(i, &c)| chars[start + i] == c);

                    if matches_surface {
                        matches.push(entry_idx);
                    }
                }
            }
        }

        matches
    }
}

fn build_lattice(text: &str, dict: &mut Dictionary) -> (Vec<Vec<(usize, usize)>>, Vec<char>) {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut lattice = vec![Vec::with_capacity(1024); len + 1];

    for start in 0..len {
        let matches = dict.lookup(text, start);
        for entry_idx in matches {
            let entry = dict.read_entry(entry_idx);
            let end = start + entry.surface.chars().count();
            lattice[end].push((entry_idx, start));
        }
    }

    (lattice, chars)
}

pub fn transliterate(text: &str, dict: &mut Dictionary) -> String {
    if text.is_empty() {
        return String::new();
    }

    let (lattice, chars) = build_lattice(text, dict);
    let len = chars.len();

    let mut nodes: Vec<Vec<LatticeNode>> = vec![Vec::with_capacity(1024); len + 1];
    let bos_node = LatticeNode {
        start_pos: 0,
        end_pos: 0,
        entry_idx: 0,
        cost: 0,
        prev_node: None,
    };
    nodes[0].push(bos_node);

    for pos in 1..=len {
        if lattice[pos].is_empty() {
            if !nodes[pos - 1].is_empty() {
                let prev_nodes: Vec<_> = nodes[pos - 1].iter().cloned().enumerate().collect();
                for (prev_idx, prev_node) in prev_nodes {
                    nodes[pos].push(LatticeNode {
                        start_pos: pos - 1,
                        end_pos: pos,
                        entry_idx: 0,
                        cost: prev_node.cost + 10000,
                        prev_node: Some(prev_idx),
                    });
                }
            }
            continue;
        }

        for &(entry_idx, start_pos) in &lattice[pos] {
            if nodes[start_pos].is_empty() {
                continue;
            }

            let entry = dict.read_entry(entry_idx);
            let mut best_cost = i32::MAX;
            let mut best_prev = None;

            for (prev_idx, prev_node) in nodes[start_pos].iter().enumerate() {
                let prev_right_id = if start_pos == 0 {
                    0
                } else {
                    dict.read_entry(prev_node.entry_idx).right_id
                };

                let conn_cost = dict.get_matrix_cost(prev_right_id, entry.left_id) as i32;
                let total_cost = prev_node.cost + entry.word_cost as i32 + conn_cost;

                if total_cost < best_cost {
                    best_cost = total_cost;
                    best_prev = Some(prev_idx);
                }
            }

            if best_prev.is_some() {
                nodes[pos].push(LatticeNode {
                    start_pos,
                    end_pos: pos,
                    entry_idx,
                    cost: best_cost,
                    prev_node: best_prev,
                });
            }
        }
    }

    let mut result = Vec::with_capacity(1024);
    if nodes[len].is_empty() {
        return text.to_string();
    }

    if let Some(last_node) = nodes[len].iter().min_by_key(|n| n.cost) {
        let mut current_pos = len;
        let mut current_node_idx = nodes[len]
            .iter()
            .position(|n| n.cost == last_node.cost)
            .unwrap();

        while current_pos > 0 {
            let node = &nodes[current_pos][current_node_idx];
            if node.start_pos == 0 && node.end_pos == 0 {
                break;
            }

            if node.entry_idx == 0 && node.cost >= 10000 {
                result.push(chars[node.start_pos].to_string());
            } else {
                let entry = dict.read_entry(node.entry_idx);
                result.push(entry.reading.clone());
            }

            if let Some(prev_idx) = node.prev_node {
                current_pos = node.start_pos;
                current_node_idx = prev_idx;
            } else {
                break;
            }
        }
    }

    result.reverse();
    result.join("")
}
