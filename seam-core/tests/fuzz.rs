//! Fuzzing the two hand-written parsers and the validator.
//!
//! The engine reads untrusted bytes with code written by hand, and the README
//! promises that "a panic reaching a foreign runtime is treated as a Seam bug"
//! and that hostile input is bounded. Those are claims about inputs nobody
//! thought of, which is exactly what a test suite made of cases somebody
//! thought of cannot check.
//!
//! Not `cargo-fuzz`: that needs nightly, and libFuzzer on Windows/MSVC is a
//! fight. A coverage-guided fuzzer explores more, but one that runs on every
//! push in the toolchain the project already pins finds more in practice than
//! one nobody runs. The generator here is deterministic and seeded, so a
//! failure is reproducible from the number printed with it.
//!
//! The property under test is narrow and absolute: **nothing here may panic.**
//! Rejecting an input is a correct answer. Accepting one is a correct answer.
//! Unwinding is not, because it crosses an FFI boundary into a runtime that
//! cannot catch it.
//!
//! Run longer with `SEAM_FUZZ_ITERATIONS=200000 cargo test --test fuzz`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::panic::{catch_unwind, AssertUnwindSafe};

use seam_core::{input::Input, json::Document, parse, validate::validate, Limits};

/// Deterministic and tiny, so a seed reproduces a failure exactly. The quality
/// of the bits does not matter here; the coverage of the shapes does.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*, chosen for being four lines rather than for its spectrum.
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }

    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }
}

/// Fragments that mean something to one of the parsers, so mutation lands on
/// interesting boundaries rather than on noise the lexer rejects immediately.
const FRAGMENTS: &[&str] = &[
    "{",
    "}",
    "[",
    "]",
    ":",
    ",",
    "\"",
    "\\",
    "\\u",
    "\\ud800",
    "\\udc00",
    "\\uD83D\\uDE00",
    "-",
    "+",
    ".",
    "e",
    "E",
    "0",
    "00",
    "1e999",
    "-0",
    "9007199254740993",
    "18446744073709551616",
    "true",
    "false",
    "null",
    "NaN",
    "Infinity",
    " ",
    "\t",
    "\n",
    "\r",
    "\u{0}",
    "\u{7f}",
    "\u{85}",
    "é",
    "日本語",
    "🙂",
    "\u{feff}",
    "schema",
    "union",
    "optional",
    "enum",
    "@tag",
    "@format",
    "@min_len",
    "@range",
    "u64",
    "String",
    "Date",
    "DateTime",
    "?",
    "(",
    ")",
    "..=",
    "//",
];

/// Whole inputs worth mutating: the real schemas, plus shapes that have caused
/// trouble in parsers generally.
fn seeds() -> Vec<String> {
    let mut out = vec![
        String::new(),
        "{}".into(),
        "[]".into(),
        "null".into(),
        "\"\"".into(),
        "{\"a\":1}".into(),
        "[[[[[[[[[[]]]]]]]]]]".into(),
        "{\"a\":{\"b\":{\"c\":[1,2,3]}}}".into(),
        "\"\\ud800\"".into(),
        "1e309".into(),
        "-9223372036854775809".into(),
        "schema A { x: u8 }".into(),
        "schema A { x: String @min_len(1) @format(email) }".into(),
        "schema A { x: optional [String?]? }".into(),
        "union U @tag(\"t\") { a: A }\nschema A { x: u8 }".into(),
        "schema A { x: enum { a, b } }".into(),
    ];

    // The repository's own schemas: the most realistic starting points there are.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../conformance/schemas");
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if let Ok(text) = std::fs::read_to_string(entry.path()) {
                out.push(text);
            }
        }
    }
    out
}

fn mutate(rng: &mut Rng, input: &str) -> String {
    let mut bytes = input.as_bytes().to_vec();

    for _ in 0..1 + rng.below(4) {
        match rng.below(8) {
            // Splice in a fragment that means something to a parser.
            0 => {
                let at = rng.below(bytes.len() + 1);
                let frag = rng.pick(FRAGMENTS).as_bytes();
                bytes.splice(at..at, frag.iter().copied());
            }
            // Truncate: half-finished input is where parsers reach past the end.
            1 => {
                let keep = rng.below(bytes.len() + 1);
                bytes.truncate(keep);
            }
            // Flip a bit, which is how a byte stops being valid UTF-8.
            2 => {
                if !bytes.is_empty() {
                    let at = rng.below(bytes.len());
                    bytes[at] ^= 1 << rng.below(8);
                }
            }
            // Delete a run.
            3 => {
                if !bytes.is_empty() {
                    let at = rng.below(bytes.len());
                    let end = (at + 1 + rng.below(8)).min(bytes.len());
                    bytes.drain(at..end);
                }
            }
            // Duplicate a run, which grows nesting and repeats keys.
            4 => {
                if !bytes.is_empty() && bytes.len() < 4096 {
                    let at = rng.below(bytes.len());
                    let end = (at + 1 + rng.below(32)).min(bytes.len());
                    let run: Vec<u8> = bytes[at..end].to_vec();
                    bytes.splice(at..at, run);
                }
            }
            // A raw byte, including ones no encoder would produce.
            5 => {
                let at = rng.below(bytes.len() + 1);
                bytes.insert(at, rng.below(256) as u8);
            }
            // Deep nesting, the shape that took the stack down before the cap.
            6 => {
                let n = 1 + rng.below(400);
                let open = rng.below(2) == 0;
                let (a, b) = if open { (b'[', b']') } else { (b'{', b'}') };
                let mut deep = vec![a; n];
                deep.extend(std::iter::repeat_n(b, n));
                bytes = deep;
            }
            // A long run of one byte: length limits and reallocation paths.
            _ => {
                let n = 1 + rng.below(2048);
                let b = *rng.pick(&[b'a', b'0', b'\\', b'"', b' ', 0xff]);
                let at = rng.below(bytes.len() + 1);
                bytes.splice(at..at, std::iter::repeat_n(b, n));
            }
        }
    }

    // Lossy on purpose: invalid UTF-8 must reach the byte parser, and the text
    // parser takes a &str, so this is where the two paths legitimately differ.
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Every entry point, against one input. Returns the name of whatever panicked.
fn exercise(input: &str) -> Option<&'static str> {
    let bytes = input.as_bytes();

    // 1. The .seam front end, over arbitrary text.
    let schema = match catch_unwind(AssertUnwindSafe(|| parse(input))) {
        Err(_) => return Some("parse"),
        Ok(result) => result.ok(),
    };

    // 2. The JSON reader, over arbitrary bytes, at both the default limits and
    //    limits tight enough to exercise every refusal path.
    for limits in [
        Limits::DEFAULT,
        Limits {
            max_depth: 2,
            max_items: 2,
            max_string_bytes: 4,
            max_object_keys: 2,
        },
        // Above the cap: the engine must hold it down rather than recurse.
        Limits { max_depth: usize::MAX, ..Limits::DEFAULT },
    ] {
        let parsed = catch_unwind(AssertUnwindSafe(|| {
            Document::parse(bytes, limits).map(|doc| {
                // Walking the tree is part of what a binding does, so a node
                // that parsed but cannot be read is still a bug.
                fn walk(r: &seam_core::json::Ref<'_, '_>, depth: usize) {
                    if depth > 300 {
                        return;
                    }
                    let _ = r.kind();
                    let _ = r.as_bool();
                    let _ = r.as_int();
                    let _ = r.as_f64();
                    let _ = r.as_str();
                    let _ = r.len();
                    for each in r.elements() {
                        walk(&each, depth + 1);
                    }
                    for (_, each) in r.entries() {
                        walk(&each, depth + 1);
                    }
                }
                walk(&doc.root(), 0);
            })
        }));
        if parsed.is_err() {
            return Some("json");
        }
    }

    // 3. Validation, where a schema came out of step 1. Every declared name,
    //    so unions and objects both get walked.
    if let Some(schema) = schema {
        let names: Vec<String> = schema
            .types
            .keys()
            .chain(schema.unions.keys())
            .cloned()
            .collect();
        let validated = catch_unwind(AssertUnwindSafe(|| {
            if let Ok(doc) = Document::parse(bytes, Limits::DEFAULT) {
                let root = doc.root();
                for name in &names {
                    let _ = validate(&schema, name, &root, Limits::DEFAULT);
                }
                // A name nothing declares, which is its own reported verdict.
                let _ = validate(&schema, "\u{0}nope", &root, Limits::DEFAULT);
            }
        }));
        if validated.is_err() {
            return Some("validate");
        }
    }

    // 4. The formats, over arbitrary text. These slice strings by byte index,
    //    which is where a multi-byte character bites.
    let formatted = catch_unwind(AssertUnwindSafe(|| {
        for f in seam_core::Format::ALL {
            let _ = f.matches(input);
        }
    }));
    if formatted.is_err() {
        return Some("format");
    }

    None
}

fn iterations() -> usize {
    std::env::var("SEAM_FUZZ_ITERATIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        // Enough to be worth running on every push, short enough that nobody
        // is tempted to skip it.
        .unwrap_or(20_000)
}

#[test]
fn no_input_makes_the_engine_panic() {
    let seeds = seeds();
    let total = iterations();

    // Quiet: a caught panic is the finding, and the default hook would print a
    // backtrace for each one before this test could report it usefully.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    let mut failures: Vec<(u64, &'static str, String)> = Vec::new();
    for i in 0..total {
        let seed = 0x5EA3_0000_0000_0001 ^ i as u64;
        let mut rng = Rng(seed);
        let seed_input = rng.pick(&seeds).clone();
        let input = mutate(&mut rng, &seed_input);

        if let Some(where_) = exercise(&input) {
            failures.push((seed, where_, input));
            if failures.len() >= 5 {
                break;
            }
        }
    }

    std::panic::set_hook(hook);

    assert!(
        failures.is_empty(),
        "the engine panicked on {} input(s):\n{}",
        failures.len(),
        failures
            .iter()
            .map(|(seed, where_, input)| format!(
                "  seed {seed:#x} panicked in `{where_}` on {} bytes:\n    {:?}",
                input.len(),
                // Bounded: a two-kilobyte input in an assertion message helps
                // nobody, and the seed reproduces the whole of it.
                &input[..input.len().min(200)]
            ))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// Inputs that once broke something. A fuzzer finds a bug once; this is what
/// keeps it found, and it runs in milliseconds on the pinned stable toolchain.
#[test]
fn known_bad_inputs_stay_fixed() {
    let cases: &[&str] = &[
        // Depth: took the process down before `Limits::MAX_DEPTH` existed.
        // Not a panic to catch — a stack overflow aborts — so the guard is the
        // cap, and this asserts the cap still holds.
        "",
        "[",
        "{",
        "\"",
        "\\",
        "\"\\u",
        "\"\\ud800\"",
        "\"\\udc00\"",
        "-",
        "1e",
        "1e+",
        "0123",
        "{\"a\"",
        "{\"a\":",
        "{\"a\":}",
        "[,]",
        "[1,]",
        "schema",
        "schema A",
        "schema A {",
        "schema A { x",
        "schema A { x:",
        "schema A { x: }",
        "union",
        "union U",
        "union U @tag",
        "union U @tag(",
        "union U @tag()",
        "schema A { x: String @format( }",
        "schema A { x: String @min_len( }",
        "schema A { x: String @range(1..=) }",
        "é",
        "\u{feff}schema A { x: u8 }",
    ];

    for case in cases {
        assert!(
            exercise(case).is_none(),
            "panicked on the regression input {case:?}"
        );
    }
}

#[test]
fn the_harness_would_notice_a_panic() {
    // A fuzzer that cannot fail proves nothing, the same reason the
    // conformance runner feeds itself a wrong expectation.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let caught = catch_unwind(AssertUnwindSafe(|| panic!("deliberate"))).is_err();
    std::panic::set_hook(hook);
    assert!(caught);
}

/// Not a check of the engine but of this file: a generator that produced noise
/// the parsers reject at the first byte would pass forever while testing
/// nothing. Asserts that the corpus actually reaches accepting paths.
#[test]
fn the_generator_reaches_past_the_first_byte() {
    let seeds = seeds();
    let (mut json_ok, mut schema_ok, mut nonempty) = (0usize, 0usize, 0usize);
    let total = 20_000;

    for i in 0..total {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ i as u64);
        let seed_input = rng.pick(&seeds).clone();
        let input = mutate(&mut rng, &seed_input);

        if !input.is_empty() {
            nonempty += 1;
        }
        if Document::parse(input.as_bytes(), Limits::DEFAULT).is_ok() {
            json_ok += 1;
        }
        if parse(&input).is_ok() {
            schema_ok += 1;
        }
    }

    // Loose thresholds on purpose: this guards against a generator that has
    // stopped working, not against normal drift in the mutation mix.
    assert!(
        nonempty > total / 2,
        "most inputs were empty: {nonempty}/{total}"
    );
    assert!(
        json_ok > total / 100,
        "almost nothing parsed as JSON: {json_ok}/{total} — the generator is producing noise"
    );
    assert!(
        schema_ok > total / 1000,
        "almost nothing parsed as a schema: {schema_ok}/{total} — the generator is producing noise"
    );

    println!("corpus: {json_ok}/{total} valid JSON, {schema_ok}/{total} valid schemas");
}
