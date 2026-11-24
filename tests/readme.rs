//! The README's quick start is `examples/counter.rs`, which clippy compiles.

#[test]
fn quick_start_is_the_example() {
    let example = include_str!("../examples/counter.rs");
    let code: String = example
        .lines()
        .skip_while(|line| line.starts_with("//!") || line.is_empty())
        .flat_map(|line| [line, "\n"])
        .collect();
    assert!(
        include_str!("../README.md").contains(&format!("```rust\n{code}```")),
        "README.md's quick start differs from examples/counter.rs"
    );
}
