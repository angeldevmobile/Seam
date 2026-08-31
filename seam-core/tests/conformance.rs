//! Runs the shared conformance suite in `conformance/`.
//!
//! This is the reference runner. Every binding must implement the same one and
//! agree with it case for case; that agreement is what "no drift" means.
//!
//! Note that lowering JSON into a `Value` is part of the job under test, not a
//! detail of the harness. A binding that corrupts an integer on the way in is
//! non-conformant even if the validator it calls is perfect, so `integer_too_wide`
//! is reported here, during lowering, exactly as `seam-py` will have to.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::{Path as FsPath, PathBuf};

use seam_core::{
    error::{Code, Segment},
    parse,
    validate::validate,
    value::{Int, Value},
    Limits, Path, Schema,
};
use serde_json::Value as J;

fn root() -> PathBuf {
    FsPath::new(env!("CARGO_MANIFEST_DIR")).join("../conformance")
}

#[test]
fn every_case_in_the_suite_holds() {
    let cases_dir = root().join("cases");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&cases_dir)
        .unwrap_or_else(|e| panic!("reading {}: {e}", cases_dir.display()))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();

    assert!(!files.is_empty(), "the suite must not be empty");

    let mut failures = Vec::new();
    let mut ran = 0_usize;

    for file in &files {
        let label = file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        run_file(file, &label, &mut ran, &mut failures);
    }

    assert!(
        failures.is_empty(),
        "{} of {ran} conformance cases failed:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
    assert!(ran >= 68, "expected a meaningful suite, ran only {ran}");
}

/// A harness that cannot fail proves nothing, so check that it can.
#[test]
fn the_harness_detects_a_wrong_expectation() {
    let schema = load_schema("user");
    let valid = r#"{ "id": 1, "name": "Gabriel", "plan": "pro", "nickname": null }"#;

    let case = |expect: &str| -> J {
        serde_json::from_str(&format!(
            r#"{{ "name": "probe", "input": {valid}, "expect": {expect} }}"#
        ))
        .expect("probe should be valid JSON")
    };

    assert!(run_case(&schema, "User", None, &case(r#""valid""#)).is_ok());
    assert!(run_case(
        &schema,
        "User",
        None,
        &case(r#"{ "issues": [{ "path": "id", "code": "type_mismatch" }] }"#)
    )
    .is_err());
}

fn run_file(file: &FsPath, label: &str, ran: &mut usize, failures: &mut Vec<String>) {
    let text =
        std::fs::read_to_string(file).unwrap_or_else(|e| panic!("reading {}: {e}", file.display()));
    let doc: J = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("{} is not valid JSON: {e}", file.display()));

    let schema = load_schema(doc["schema"].as_str().expect("`schema` must be a string"));
    let type_name = doc["type"].as_str().expect("`type` must be a string");
    let base = doc.get("base").and_then(J::as_object).cloned();

    let cases = doc["cases"].as_array().expect("`cases` must be an array");
    for case in cases {
        *ran += 1;
        let name = case["name"].as_str().unwrap_or("<unnamed>");
        if let Err(why) = run_case(&schema, type_name, base.as_ref(), case) {
            failures.push(format!("  {label} :: {name}\n    {why}"));
        }
    }
}

fn load_schema(name: &str) -> Schema {
    let path = root().join("schemas").join(format!("{name}.seam"));
    let src = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    parse(&src).unwrap_or_else(|e| panic!("parsing {}: {e}", path.display()))
}

type Found = Vec<(String, String)>;

fn run_case(
    schema: &Schema,
    type_name: &str,
    base: Option<&serde_json::Map<String, J>>,
    case: &J,
) -> Result<(), String> {
    let mut merged = base.cloned().unwrap_or_default();
    if let Some(input) = case["input"].as_object() {
        for (k, v) in input {
            merged.insert(k.clone(), v.clone());
        }
    }

    let found = match lower(&J::Object(merged), &mut Vec::new()) {
        // Lowering failed, which is itself a conformant verdict.
        Err(issue) => vec![issue],
        Ok(value) => match validate(schema, type_name, &value, Limits::DEFAULT) {
            Ok(()) => Vec::new(),
            Err(e) => e
                .issues
                .into_iter()
                .map(|i| (i.path.render(), i.code.as_str().to_string()))
                .collect(),
        },
    };

    let expected = expected_issues(&case["expect"]);
    if found == expected {
        Ok(())
    } else {
        Err(format!("expected {expected:?}\n    found    {found:?}"))
    }
}

fn expected_issues(expect: &J) -> Found {
    match expect {
        J::String(s) if s == "valid" => Vec::new(),
        J::Object(o) => o
            .get("issues")
            .and_then(J::as_array)
            .map(|issues| {
                issues
                    .iter()
                    .map(|i| {
                        (
                            i["path"].as_str().unwrap_or_default().to_string(),
                            i["code"].as_str().unwrap_or_default().to_string(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
        other => panic!("`expect` must be \"valid\" or an object, found {other}"),
    }
}

/// JSON into a `Value`, reporting the one failure a binding can hit before the
/// validator runs: an integer too wide for the model.
fn lower(v: &J, path: &mut Vec<Segment>) -> Result<Value, (String, String)> {
    Ok(match v {
        J::Null => Value::Null,
        J::Bool(b) => Value::Bool(*b),
        J::String(s) => Value::String(s.clone()),
        J::Number(n) => number(n.as_str(), path)?,
        J::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                path.push(Segment::Index(i));
                let lowered = lower(item, path);
                path.pop();
                out.push(lowered?);
            }
            Value::Array(out)
        }
        J::Object(map) => {
            let mut out = BTreeMap::new();
            for (k, val) in map {
                path.push(Segment::Key(k.clone()));
                let lowered = lower(val, path);
                path.pop();
                out.insert(k.clone(), lowered?);
            }
            Value::Object(out)
        }
    })
}

/// `arbitrary_precision` hands back the literal, so the decision of what kind
/// of number this is stays here rather than in a `f64` that already lost it.
fn number(literal: &str, path: &[Segment]) -> Result<Value, (String, String)> {
    let fractional = literal.contains(['.', 'e', 'E']);
    if !fractional {
        if let Ok(n) = literal.parse::<i64>() {
            return Ok(Value::Int(Int::Signed(n)));
        }
        if let Ok(n) = literal.parse::<u64>() {
            return Ok(Value::Int(Int::Unsigned(n)));
        }
        // An integer that fits neither: too wide, not out of range.
        return Err((
            Path(path.to_vec()).render(),
            Code::IntegerTooWide.as_str().to_string(),
        ));
    }
    match literal.parse::<f64>() {
        Ok(f) => Ok(Value::Float(f)),
        Err(e) => panic!("`{literal}` is not a number the harness can lower: {e}"),
    }
}
