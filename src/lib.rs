use std::collections::HashMap;
use std::fs::File;
use std::io::Read;

#[derive(Debug, Clone)]
pub struct DictEntry {
    pub surface: String,
    pub pair_id: u16,
    pub word_cost: i16,
    pub reading: String,
}

#[derive(Debug)]
pub struct Dictionary {
    pub entries: Vec<DictEntry>,
    index: HashMap<char, Vec<usize>>,
    matrix: Vec<i16>,
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
    fn get_matrix_cost(&self, _prev_pair_id: u16, curr_pair_id: u16) -> i16 {
        self.matrix.get(curr_pair_id as usize).copied().unwrap_or(0)
    }

    pub fn load(path: &str) -> std::io::Result<Self> {
        let mut file = File::open(path)?;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)?;

        if buffer.len() < 16 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "File too small",
            ));
        }

        if &buffer[0..4] != b"MUCA" {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid magic number",
            ));
        }

        let _version = u16::from_le_bytes([buffer[4], buffer[5]]);
        let num_matrix = u16::from_le_bytes([buffer[6], buffer[7]]) as usize;
        let num_entries =
            u32::from_le_bytes([buffer[8], buffer[9], buffer[10], buffer[11]]) as usize;
        let index_offset =
            u32::from_le_bytes([buffer[12], buffer[13], buffer[14], buffer[15]]) as usize;

        if buffer.len() < 20 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Header too small",
            ));
        }
        let strings_offset =
            u32::from_le_bytes([buffer[16], buffer[17], buffer[18], buffer[19]]) as usize;

        let mut matrix = vec![0i16; num_matrix];
        let mut pos = 20;
        for _ in 0..num_matrix {
            let pair_id = u16::from_le_bytes([buffer[pos], buffer[pos + 1]]) as usize;
            let cost = i16::from_le_bytes([buffer[pos + 2], buffer[pos + 3]]);
            if pair_id < matrix.len() {
                matrix[pair_id] = cost;
            }
            pos += 4;
        }

        let mut entries = Vec::with_capacity(num_entries);
        for _ in 0..num_entries {
            let surf_off = u32::from_le_bytes([
                buffer[pos],
                buffer[pos + 1],
                buffer[pos + 2],
                buffer[pos + 3],
            ]) as usize;
            let surf_len = buffer[pos + 4] as usize;
            let read_off = u32::from_le_bytes([
                buffer[pos + 5],
                buffer[pos + 6],
                buffer[pos + 7],
                buffer[pos + 8],
            ]) as usize;
            let read_len = buffer[pos + 9] as usize;
            let pair_id = u16::from_le_bytes([buffer[pos + 10], buffer[pos + 11]]);
            let cost = i16::from_le_bytes([buffer[pos + 12], buffer[pos + 13]]);

            let surface_bytes =
                &buffer[strings_offset + surf_off..strings_offset + surf_off + surf_len];
            let reading_bytes =
                &buffer[strings_offset + read_off..strings_offset + read_off + read_len];

            entries.push(DictEntry {
                surface: String::from_utf8_lossy(surface_bytes).to_string(),
                pair_id,
                word_cost: cost,
                reading: String::from_utf8_lossy(reading_bytes).to_string(),
            });

            pos += 16;
        }

        let num_index_keys = u32::from_le_bytes([
            buffer[index_offset],
            buffer[index_offset + 1],
            buffer[index_offset + 2],
            buffer[index_offset + 3],
        ]) as usize;

        let mut index: HashMap<char, Vec<usize>> = HashMap::new();
        pos = index_offset + 4;
        for _ in 0..num_index_keys {
            let ch = char::from_u32(u32::from_le_bytes([
                buffer[pos],
                buffer[pos + 1],
                buffer[pos + 2],
                buffer[pos + 3],
            ]))
            .unwrap();
            let count = u16::from_le_bytes([buffer[pos + 4], buffer[pos + 5]]) as usize;
            pos += 8;

            let mut entry_ids = Vec::with_capacity(count);
            for _ in 0..count {
                let entry_id = u32::from_le_bytes([
                    buffer[pos],
                    buffer[pos + 1],
                    buffer[pos + 2],
                    buffer[pos + 3],
                ]) as usize;
                entry_ids.push(entry_id);
                pos += 4;
            }
            index.insert(ch, entry_ids);
        }

        Ok(Dictionary {
            entries,
            index,
            matrix,
        })
    }

    fn lookup(&self, text: &str, start: usize) -> Vec<usize> {
        let chars: Vec<char> = text.chars().collect();
        if start >= chars.len() {
            return Vec::with_capacity(1024);
        }

        let first_char = chars[start];
        let mut matches = Vec::with_capacity(1024);

        if let Some(candidates) = self.index.get(&first_char) {
            for &entry_idx in candidates {
                let entry = &self.entries[entry_idx];
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

fn build_lattice(text: &str, dict: &Dictionary) -> (Vec<Vec<(usize, usize)>>, Vec<char>) {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut lattice = vec![Vec::with_capacity(1024); len + 1];

    for start in 0..len {
        let matches = dict.lookup(text, start);
        for entry_idx in matches {
            let entry = &dict.entries[entry_idx];
            let end = start + entry.surface.chars().count();
            lattice[end].push((entry_idx, start));
        }
    }

    (lattice, chars)
}

pub fn transliterate(text: &str, dict: &Dictionary) -> String {
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

            let entry = &dict.entries[entry_idx];
            let mut best_cost = i32::MAX;
            let mut best_prev = None;

            for (prev_idx, prev_node) in nodes[start_pos].iter().enumerate() {
                let prev_pair_id = if start_pos == 0 {
                    0
                } else {
                    dict.entries[prev_node.entry_idx].pair_id
                };

                let conn_cost = dict.get_matrix_cost(prev_pair_id, entry.pair_id) as i32;
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
                let entry = &dict.entries[node.entry_idx];
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
