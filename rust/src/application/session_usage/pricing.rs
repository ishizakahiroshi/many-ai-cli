const PRICES: &[(&str, f64, f64, f64)] = &[
    ("gpt-4.1", 2.00, 8.00, 0.50),
    ("gpt-4.1-mini", 0.40, 1.60, 0.10),
    ("gpt-4.1-nano", 0.10, 0.40, 0.025),
    ("gpt-4o", 2.50, 10.00, 1.25),
    ("gpt-4o-mini", 0.15, 0.60, 0.075),
    ("gpt-5", 10.00, 40.00, 2.50),
    ("gpt-5.5", 10.00, 40.00, 2.50),
    ("o3", 10.00, 40.00, 2.50),
    ("o4-mini", 1.10, 4.40, 0.275),
    ("claude-opus-4", 15.00, 75.00, 1.50),
    ("claude-sonnet-4", 3.00, 15.00, 0.30),
    ("claude-haiku-4", 0.80, 4.00, 0.08),
    ("claude-fable-5", 10.00, 50.00, 1.00),
    ("claude-opus-4-8", 5.00, 25.00, 0.50),
    ("claude-opus-4-7", 5.00, 25.00, 0.50),
    ("claude-opus-4-6", 5.00, 25.00, 0.50),
    ("claude-sonnet-4-6", 3.00, 15.00, 0.30),
    ("claude-opus-4-5", 15.00, 75.00, 1.50),
    ("claude-sonnet-4-5", 3.00, 15.00, 0.30),
    ("claude-haiku-4-5", 1.00, 5.00, 0.10),
    ("claude-3-5-sonnet", 3.00, 15.00, 0.30),
    ("claude-3-5-haiku", 0.80, 4.00, 0.08),
    ("claude-3-opus", 15.00, 75.00, 1.50),
    ("claude-3-sonnet", 3.00, 15.00, 0.30),
    ("claude-3-haiku", 0.25, 1.25, 0.03),
];
pub fn cost(model: &str, input: i64, output: i64, cache: i64) -> (f64, bool) {
    let find = |name: &str| PRICES.iter().find(|v| v.0 == name);
    let price = find(model).or_else(|| model.split_once(' ').and_then(|(first, _)| find(first)));
    let Some((_, i, o, c)) = price else {
        return (0.0, false);
    };
    let input = input.max(0);
    let output = output.max(0);
    let cache = cache.max(0).min(input);
    (
        (input - cache) as f64 * i / 1_000_000.0
            + output as f64 * o / 1_000_000.0
            + cache as f64 * c / 1_000_000.0,
        true,
    )
}
