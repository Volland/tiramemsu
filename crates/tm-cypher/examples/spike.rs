fn main() {
    let path = std::env::args().nth(1).unwrap();
    for q in std::fs::read_to_string(path).unwrap().lines() {
        match tm_cypher::parse::adapter::parse(q) {
            Ok(_) => println!("OK    {q}"),
            Err(e) => println!("ERR   {q}\n      => {e:?}"),
        }
    }
}
