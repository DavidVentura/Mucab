use mucab::{transliterate, Dictionary};
use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: {} <mucab.bin> <text>", args[0]);
        std::process::exit(1);
    }

    let dict_path = &args[1];
    let input_text = &args[2];

    let dict = Dictionary::load(dict_path).expect("Failed to load dictionary");
    println!("Loaded {} dictionary entries", dict.entries.len());

    println!("Input: {}", input_text);

    let result = transliterate(input_text, &dict);
    println!("Output: {}", result);
}
