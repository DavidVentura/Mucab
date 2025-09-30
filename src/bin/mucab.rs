use mucab::{transliterate, ConnectionMatrix, Dictionary};
use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 4 {
        eprintln!("Usage: {} <dictionary.csv> <matrix.def> <text>", args[0]);
        std::process::exit(1);
    }

    let dict_path = &args[1];
    let matrix_path = &args[2];
    let input_text = &args[3];

    let dict = Dictionary::load(dict_path).expect("Failed to load dictionary");
    println!("Loaded {} dictionary entries", dict.entries.len());

    let matrix = ConnectionMatrix::load(matrix_path).expect("Failed to load matrix");
    println!("Loaded connection matrix entries");

    println!("Input: {}", input_text);

    let result = transliterate(input_text, &dict, &matrix);
    println!("Output: {}", result);
}