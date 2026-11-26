//! cargo run -p fuzzrank --release --example bench
use std::hint::black_box;
use std::time::Instant;

use fuzzrank::Index;

fn main() {
    let words = ["deployment", "pipeline", "budget", "approval", "rollback", "substation", "transformer", "meter", "invoice", "petty", "cash", "release"];
    let lines: Vec<String> = (0..50_000)
        .map(|i| format!("{} {} {} #{i}", words[i % 12], words[(i * 7) % 12], words[(i * 13) % 12]))
        .collect();

    let t = Instant::now();
    let index = Index::new(&lines);
    println!("index {} lines: {:?}", index.len(), t.elapsed());

    for q in ["dep roll", "substation", "zzz", "petty cash 4999", "e"] {
        let runs = 50;
        let t = Instant::now();
        let mut hits = 0;
        for _ in 0..runs {
            hits = black_box(index.search(black_box(q), 10)).len();
        }
        println!("search {q:>16}: {:>8.2?} per query, {hits} shown", t.elapsed() / runs);
    }
}
