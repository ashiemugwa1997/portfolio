//! cargo run -p fuzzrank --example palette -- "dep roll"
use fuzzrank::Index;

fn main() {
    let lines = [
        "Deployment pipeline with automatic rollback",
        "Budgeting module: multi-level approval",
        "SAML single sign-on",
        "Drone Training Platform in React Native",
    ];
    let query = std::env::args().nth(1).unwrap_or_else(|| "dep roll".into());
    let index = Index::new(lines);
    for m in index.search(&query, 5) {
        println!("{:>6}  {}", m.score, lines[m.index]);
    }
}
