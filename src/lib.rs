use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};

#[derive(Debug, Clone)]
pub struct DictEntry {
    pub surface: String,
    pub pair_id: u16,
    pub word_cost: i32,
    pub reading: String,
}

#[derive(Debug)]
pub struct Dictionary {
    pub entries: Vec<DictEntry>,
    index: HashMap<char, Vec<usize>>,
}

pub struct ConnectionMatrix {
    data: HashMap<u16, i16>,
}

#[derive(Debug, Clone)]
struct LatticeNode {
    start_pos: usize,
    end_pos: usize,
    entry_idx: usize,
    cost: i32,
    prev_node: Option<usize>,
}

impl ConnectionMatrix {
    pub fn load(path: &str) -> std::io::Result<Self> {
        let mut file = File::open(path)?;
        let mut data = HashMap::new();

        let mut buffer = [0u8; 4];
        while file.read_exact(&mut buffer).is_ok() {
            let pair_id = u16::from_le_bytes([buffer[0], buffer[1]]);
            let cost = i16::from_le_bytes([buffer[2], buffer[3]]);
            data.insert(pair_id, cost);
        }

        Ok(ConnectionMatrix { data })
    }

    fn get_cost(&self, _prev_pair_id: u16, curr_pair_id: u16) -> i16 {
        *self.data.get(&curr_pair_id).unwrap_or(&0)
    }
}

impl Dictionary {
    pub fn load(path: &str) -> std::io::Result<Self> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);

        let mut entries = Vec::with_capacity(1024);
        let mut index: HashMap<char, Vec<usize>> = HashMap::new();

        for line in reader.lines() {
            let line = line?;
            let parts: Vec<&str> = line.split(',').collect();

            if parts.len() < 4 {
                continue;
            }

            let surface = parts[0].to_string();
            let pair_id = parts[1].parse().unwrap_or(0);
            let word_cost = parts[2].parse().unwrap_or(0);
            let reading = parts[3].to_string();

            let entry_idx = entries.len();
            entries.push(DictEntry {
                surface: surface.clone(),
                pair_id,
                word_cost,
                reading,
            });

            if let Some(first_char) = surface.chars().next() {
                index.entry(first_char).or_default().push(entry_idx);
            }
        }

        Ok(Dictionary { entries, index })
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

pub fn transliterate(text: &str, dict: &Dictionary, matrix: &ConnectionMatrix) -> String {
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

                let conn_cost = matrix.get_cost(prev_pair_id, entry.pair_id) as i32;
                let total_cost = prev_node.cost + entry.word_cost + conn_cost;

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

