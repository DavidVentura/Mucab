use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use zeekstd::Decoder;

const HEADER_SIZE: usize = 16;
const ENTRY_METADATA_SIZE: usize = 9;
const DEFAULT_CAPACITY: usize = 1024;

type Lattice = Vec<Vec<((char, usize), usize)>>;

struct OffsetFile<R: Read + Seek> {
    reader: R,
    base_offset: u64,
}

impl<R: Read + Seek> OffsetFile<R> {
    fn new(mut r: R, base_offset: u64) -> std::io::Result<Self> {
        r.seek(SeekFrom::Start(base_offset))?;
        Ok(Self {
            reader: r,
            base_offset,
        })
    }
}

impl<R: Read + Seek> Read for OffsetFile<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.reader.read(buf)
    }
}

impl<R: Read + Seek> Seek for OffsetFile<R> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let adjusted_pos = match pos {
            SeekFrom::Start(offset) => SeekFrom::Start(self.base_offset + offset),
            SeekFrom::Current(offset) => SeekFrom::Current(offset),
            SeekFrom::End(offset) => SeekFrom::End(offset),
        };
        let result = self.reader.seek(adjusted_pos)?;
        Ok(result - self.base_offset)
    }
}

#[derive(Debug, Clone)]
pub struct DictEntry {
    pub surface: String,
    pub pos_id: u16,
    pub word_cost: i16,
    pub reading_offset: u32,
    pub reading_len: u8,
}

pub struct Dictionary<'a> {
    decoder: Decoder<'a, OffsetFile<BufReader<File>>>,
    strings_offset: u64,
    pub num_entries: usize,
    index: HashMap<char, (u64, usize)>,
    entry_cache: HashMap<char, Vec<DictEntry>>,
    matrix: Vec<i16>,
    matrix_size: usize,
}

#[derive(Debug, Clone)]
struct LatticeNode {
    start_pos: usize,
    end_pos: usize,
    entry_char: char,
    entry_local_idx: usize,
    cost: i32,
    prev_node: Option<usize>,
}

impl<'a> Dictionary<'a> {
    fn get_matrix_cost(&self, prev_id: u16, curr_id: u16) -> i16 {
        let idx = (prev_id as usize) * self.matrix_size + (curr_id as usize);
        self.matrix.get(idx).copied().unwrap_or(0)
    }

    fn get_entry(&self, first_char: char, local_idx: usize) -> &DictEntry {
        &self.entry_cache[&first_char][local_idx]
    }

    fn read_reading_at(&mut self, offset: u32, len: u8) -> String {
        let start = self.strings_offset + offset as u64;
        let end = start + len as u64;
        self.decoder.set_offset(start).unwrap();
        self.decoder.set_offset_limit(end).unwrap();
        let mut reading_bytes = vec![0u8; len as usize];
        self.decoder.read_exact(&mut reading_bytes).unwrap();
        String::from_utf8(reading_bytes).unwrap()
    }

    fn bulk_read_entries(&mut self, first_char: char) -> Vec<DictEntry> {
        let (byte_offset, count) = *self.index.get(&first_char).unwrap();
        let mut entries = Vec::with_capacity(count);

        self.decoder.set_offset(byte_offset).unwrap();

        for _ in 0..count {
            let mut surf_len = 0u8;
            self.decoder.read_exact(std::slice::from_mut(&mut surf_len)).unwrap();
            let surf_len = surf_len as usize;

            let mut surf_bytes = vec![0u8; surf_len];
            self.decoder.read_exact(&mut surf_bytes).unwrap();

            let mut entry_buf = [0u8; ENTRY_METADATA_SIZE];
            self.decoder.read_exact(&mut entry_buf).unwrap();

            let read_off =
                u32::from_le_bytes([entry_buf[0], entry_buf[1], entry_buf[2], entry_buf[3]]);
            let read_len = entry_buf[4];
            let pos_id = u16::from_le_bytes([entry_buf[5], entry_buf[6]]);
            let cost = i16::from_le_bytes([entry_buf[7], entry_buf[8]]);

            entries.push(DictEntry {
                surface: String::from_utf8(surf_bytes).unwrap(),
                pos_id,
                word_cost: cost,
                reading_offset: read_off,
                reading_len: read_len,
            });
        }

        entries
    }

    pub fn load(path: &str) -> std::io::Result<Self> {
        let mut file = BufReader::new(File::open(path)?);

        let mut header = [0u8; HEADER_SIZE];
        file.read_exact(&mut header)?;

        if &header[0..4] != b"MUCA" {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid magic number",
            ));
        }

        let matrix_size = u16::from_le_bytes([header[6], header[7]]) as usize;
        let num_entries =
            u32::from_le_bytes([header[8], header[9], header[10], header[11]]) as usize;
        let strings_offset =
            u32::from_le_bytes([header[12], header[13], header[14], header[15]]) as u64;

        // Read matrix
        let matrix_elements = matrix_size * matrix_size;
        let mut matrix_bytes = vec![0u8; matrix_elements * 2];
        file.read_exact(&mut matrix_bytes)?;

        let mut matrix = vec![0i16; matrix_elements];
        for i in 0..matrix_elements {
            matrix[i] = i16::from_le_bytes([matrix_bytes[i * 2], matrix_bytes[i * 2 + 1]]);
        }

        // Read index immediately after matrix (no seek needed)
        let mut index_count_buf = [0u8; 4];
        file.read_exact(&mut index_count_buf)?;
        let num_index_keys = u32::from_le_bytes(index_count_buf) as usize;

        let mut index: HashMap<char, (u64, usize)> = HashMap::new();

        for _ in 0..num_index_keys {
            let mut char_buf = [0u8; 4];
            file.read_exact(&mut char_buf)?;
            let ch = char::from_u32(u32::from_le_bytes(char_buf)).unwrap();

            let mut offset_buf = [0u8; 4];
            file.read_exact(&mut offset_buf)?;
            let byte_offset = u32::from_le_bytes(offset_buf) as u64;

            let mut count_buf = [0u8; 2];
            file.read_exact(&mut count_buf)?;
            let count = u16::from_le_bytes(count_buf) as usize;

            index.insert(ch, (byte_offset, count));
        }

        let compressed_start = file.stream_position()?;
        let offset_file = OffsetFile::new(file, compressed_start)?;
        let decoder = Decoder::new(offset_file).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("zeekstd error: {:?}", e),
            )
        })?;

        Ok(Dictionary {
            decoder,
            strings_offset,
            num_entries,
            index,
            entry_cache: HashMap::new(),
            matrix,
            matrix_size,
        })
    }

    fn lookup(&mut self, text: &str, start: usize) -> Vec<(char, usize)> {
        let chars: Vec<char> = text.chars().collect();
        if start >= chars.len() {
            return Vec::with_capacity(DEFAULT_CAPACITY);
        }

        let first_char = chars[start];
        let mut matches = Vec::with_capacity(DEFAULT_CAPACITY);

        if !self.index.contains_key(&first_char) {
            return matches;
        }

        if !self.entry_cache.contains_key(&first_char) {
            let entries = self.bulk_read_entries(first_char);
            self.entry_cache.insert(first_char, entries);
        }

        let cached_entries = &self.entry_cache[&first_char];

        for (i, entry) in cached_entries.iter().enumerate() {
            let entry_chars: Vec<char> = entry.surface.chars().collect();

            if start + entry_chars.len() <= chars.len() {
                let matches_surface = entry_chars
                    .iter()
                    .enumerate()
                    .all(|(j, &c)| chars[start + j] == c);

                if matches_surface {
                    matches.push((first_char, i));
                }
            }
        }

        matches
    }
}

fn build_lattice<'a>(
    text: &str,
    dict: &mut Dictionary<'a>,
) -> (Lattice, Vec<char>) {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut lattice = vec![Vec::with_capacity(DEFAULT_CAPACITY); len + 1];

    for start in 0..len {
        let matches = dict.lookup(text, start);
        for (entry_char, entry_local_idx) in matches {
            let entry = dict.get_entry(entry_char, entry_local_idx);
            let end = start + entry.surface.chars().count();
            lattice[end].push(((entry_char, entry_local_idx), start));
        }
    }

    (lattice, chars)
}

pub fn transliterate<'a>(text: &str, dict: &mut Dictionary<'a>) -> String {
    if text.is_empty() {
        return String::new();
    }

    let (lattice, chars) = build_lattice(text, dict);
    let len = chars.len();

    let mut nodes: Vec<Vec<LatticeNode>> = vec![Vec::with_capacity(DEFAULT_CAPACITY); len + 1];
    let bos_node = LatticeNode {
        start_pos: 0,
        end_pos: 0,
        entry_char: '\0',
        entry_local_idx: 0,
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
                        entry_char: '\0',
                        entry_local_idx: 0,
                        cost: prev_node.cost + 10000,
                        prev_node: Some(prev_idx),
                    });
                }
            }
            continue;
        }

        for &((entry_char, entry_local_idx), start_pos) in &lattice[pos] {
            if nodes[start_pos].is_empty() {
                continue;
            }

            let entry = dict.get_entry(entry_char, entry_local_idx);
            let mut best_cost = i32::MAX;
            let mut best_prev = None;

            for (prev_idx, prev_node) in nodes[start_pos].iter().enumerate() {
                let prev_pos_id = if start_pos == 0 || prev_node.entry_char == '\0' {
                    0
                } else {
                    dict.get_entry(prev_node.entry_char, prev_node.entry_local_idx)
                        .pos_id
                };

                let conn_cost = dict.get_matrix_cost(prev_pos_id, entry.pos_id) as i32;
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
                    entry_char,
                    entry_local_idx,
                    cost: best_cost,
                    prev_node: best_prev,
                });
            }
        }
    }

    let mut result = Vec::with_capacity(DEFAULT_CAPACITY);
    if nodes[len].is_empty() {
        return text.to_string();
    }

    if let Some((current_node_idx, _)) = nodes[len].iter().enumerate().min_by_key(|(_, n)| n.cost) {
        let mut current_pos = len;
        let mut current_node_idx = current_node_idx;

        while current_pos > 0 {
            let node = &nodes[current_pos][current_node_idx];
            if node.start_pos == 0 && node.end_pos == 0 {
                break;
            }

            if node.entry_char == '\0' && node.cost >= 10000 {
                result.push(chars[node.start_pos].to_string());
            } else {
                let entry = dict.get_entry(node.entry_char, node.entry_local_idx);
                let read_off = entry.reading_offset;
                let read_len = entry.reading_len;
                let reading = dict.read_reading_at(read_off, read_len);
                result.push(reading);
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
