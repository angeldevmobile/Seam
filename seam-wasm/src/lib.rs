//! WebAssembly binding. Translation only: hand the engine the bytes, return or
//! report. No rule logic belongs in this file.
//!
//! One thing is decided here and it is the reason this binding exists at all:
//! **the browser is handed bytes, never an already-parsed object.** By the time
//! `JSON.parse` has run, an integer past 2^53 is already the wrong number and
//! nothing recovers it. Accepting a parsed object here would offer, in the one
//! runtime where the problem is worst, exactly the path Seam exists to avoid.
//!
//! So there is no `Input` implementation over `JsValue`. The core already reads
//! a JSON document in place, and this file only writes the result back out.

use js_sys::{Array, BigInt as JsBigInt, Date as JsDate, Object, Reflect};
use wasm_bindgen::prelude::*;

use seam_core::input::{Input, Kind};
use seam_core::json::Ref as JsonRef;
use seam_core::schema::{IntWidth, ObjectType, Type, UnionType};
use seam_core::value::{Int, Slot};

/// Reads one optional `u32` off a JS options object.
///
/// A missing or undefined key keeps the engine's default. A key that is present
/// but not a number is a mistake worth reporting rather than silently ignoring:
/// a limit that did not take effect is the kind of thing found in an incident
/// review rather than in a test.
fn limit(options: &JsValue, key: &str, default: usize) -> Result<usize, JsValue> {
    if options.is_undefined() || options.is_null() {
        return Ok(default);
    }
    let found = Reflect::get(options, &JsValue::from_str(key))?;
    if found.is_undefined() || found.is_null() {
        return Ok(default);
    }
    match found.as_f64() {
        Some(n) if n.is_finite() && n >= 1.0 => Ok(n as usize),
        _ => Err(JsError::new(&format!("`{key}` must be a positive number")).into()),
    }
}

fn limits_from(options: &JsValue) -> Result<seam_core::Limits, JsValue> {
    let d = seam_core::Limits::DEFAULT;
    Ok(seam_core::Limits {
        max_depth: limit(options, "maxDepth", d.max_depth)?,
        max_items: limit(options, "maxItems", d.max_items)?,
        max_string_bytes: limit(options, "maxStringBytes", d.max_string_bytes)?,
        max_object_keys: limit(options, "maxObjectKeys", d.max_object_keys)?,
    })
}

#[wasm_bindgen]
pub struct Schema {
    inner: std::rc::Rc<seam_core::Schema>,
}

#[wasm_bindgen]
impl Schema {
    /// Compiles a `.seam` source.
    ///
    /// There is no `load`: a browser has no filesystem, and taking a URL here
    /// would fold fetching into validation and make the whole API a promise.
    /// Fetch the file yourself and hand over the text.
    pub fn parse(source: &str) -> Result<Schema, JsValue> {
        match seam_core::parse(source) {
            Ok(inner) => Ok(Schema { inner: std::rc::Rc::new(inner) }),
            Err(e) => Err(JsError::new(&e.to_string()).into()),
        }
    }

    /// Every declared name, objects and unions alike, sorted.
    #[wasm_bindgen(js_name = typeNames)]
    pub fn type_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .inner
            .types
            .keys()
            .chain(self.inner.unions.keys())
            .cloned()
            .collect();
        names.sort();
        names
    }

    /// Binds one type. Everything that does not depend on the payload is
    /// resolved here rather than on every call.
    pub fn validator(&self, type_name: &str, limits: &JsValue) -> Result<Validator, JsValue> {
        if !self.inner.declares(type_name) {
            return Err(
                JsError::new(&format!("schema declares no type named `{type_name}`")).into(),
            );
        }
        Ok(Validator {
            schema: std::rc::Rc::clone(&self.inner),
            type_name: type_name.to_string(),
            limits: limits_from(limits)?,
        })
    }
}

#[wasm_bindgen]
pub struct Validator {
    schema: std::rc::Rc<seam_core::Schema>,
    type_name: String,
    limits: seam_core::Limits,
}

#[wasm_bindgen]
impl Validator {
    #[wasm_bindgen(getter, js_name = typeName)]
    pub fn type_name(&self) -> String {
        self.type_name.clone()
    }

    /// Validates raw JSON.
    ///
    /// Returns `{ ok: true, value }` or `{ ok: false, issues }`. The idiomatic
    /// surface lives in `index.js`, which turns the second into a thrown
    /// `SeamValidationError`; deciding on the JavaScript side is what makes the
    /// error a real `Error` with a real stack.
    pub fn validate(&self, bytes: &[u8]) -> Result<Object, JsValue> {
        let doc = match seam_core::json::Document::parse(bytes, self.limits) {
            Ok(doc) => doc,
            Err(e) => return Err(JsError::new(&e.to_string()).into()),
        };
        let root = doc.root();

        if let Err(e) = seam_core::validate(&self.schema, &self.type_name, &root, self.limits) {
            return refused(e);
        }

        // Resolved after validating, never before: for a union the shape is
        // whatever the tag turned out to say.
        let Some((ty, tag)) = shape(&self.schema, &self.type_name, &root) else {
            return Err(JsError::new(&format!(
                "`{}` validated but its shape did not resolve",
                self.type_name
            ))
            .into());
        };

        accepted(emit(&self.schema, ty, &root, tag)?.into())
    }
}

fn accepted(value: JsValue) -> Result<Object, JsValue> {
    let out = Object::new();
    Reflect::set(&out, &JsValue::from_str("ok"), &JsValue::TRUE)?;
    Reflect::set(&out, &JsValue::from_str("value"), &value)?;
    Ok(out)
}

fn refused(e: seam_core::ValidationError) -> Result<Object, JsValue> {
    let issues = Array::new();
    for issue in e.issues {
        let one = Object::new();
        Reflect::set(
            &one,
            &JsValue::from_str("path"),
            &JsValue::from_str(&issue.path.render()),
        )?;
        Reflect::set(
            &one,
            &JsValue::from_str("code"),
            &JsValue::from_str(issue.code.as_str()),
        )?;
        Reflect::set(
            &one,
            &JsValue::from_str("message"),
            &JsValue::from_str(&issue.message),
        )?;
        issues.push(&one);
    }

    let out = Object::new();
    Reflect::set(&out, &JsValue::from_str("ok"), &JsValue::FALSE)?;
    Reflect::set(&out, &JsValue::from_str("issues"), &issues)?;
    Ok(out)
}

// -------------------------------------------------------- resolving a shape

/// The object a value is actually shaped like, and the tag key to carry over.
///
/// For a `schema` that is fixed. For a `union` it is whatever the payload's tag
/// says, which is why this takes the document: a union has no shape until a
/// value picks one.
fn shape<'s>(
    schema: &'s seam_core::Schema,
    name: &str,
    r: &JsonRef<'_, '_>,
) -> Option<(&'s ObjectType, Option<&'s str>)> {
    if let Some(obj) = schema.get(name) {
        return Some((obj, None));
    }
    let u: &UnionType = schema.union(name)?;
    let found = match r.slot(&u.tag) {
        Slot::Present(v) => v.as_str()?.into_owned(),
        _ => return None,
    };
    let variant = u.variant(&found)?;
    schema
        .get(&variant.type_name)
        .map(|obj| (obj, Some(u.tag.as_str())))
}

// ------------------------------------------------------------- writing JS

fn emit(
    schema: &seam_core::Schema,
    ty: &ObjectType,
    r: &JsonRef<'_, '_>,
    tag: Option<&str>,
) -> Result<Object, JsValue> {
    let out = Object::new();

    // The tag belongs to the union rather than to the variant, so no field
    // would copy it. Dropping it would hand back an object whose own
    // discriminant is missing, which is not the value that was validated.
    if let Some(tag) = tag {
        if let Slot::Present(v) = r.slot(tag) {
            Reflect::set(&out, &JsValue::from_str(tag), &plain(&v)?)?;
        }
    }

    for field in &ty.fields {
        // Absent stays absent: the property is simply not created, which is
        // what `"bio" in user` and `JSON.stringify` both read as not sent.
        let value = match r.slot(&field.name) {
            Slot::Absent => continue,
            Slot::Null => JsValue::NULL,
            Slot::Present(v) => value(schema, &field.ty, &v)?,
        };
        Reflect::set(&out, &JsValue::from_str(&field.name), &value)?;
    }
    Ok(out)
}

fn value(schema: &seam_core::Schema, ty: &Type, r: &JsonRef<'_, '_>) -> Result<JsValue, JsValue> {
    match (ty, r.kind()) {
        // An instant becomes a real `Date`. A calendar date does not: JS has no
        // date-only type, and pushing one through an instant is what produces
        // off-by-one-day bugs, so `Date` stays the ISO string it arrived as.
        (Type::DateTime, Kind::String) => {
            let text = r.as_str().unwrap_or_default().into_owned();
            Ok(JsDate::new(&JsValue::from_str(&text)).into())
        }
        (Type::Object(obj), _) => Ok(emit(schema, obj, r, None)?.into()),
        (Type::Ref(name), _) => match shape(schema, name, r) {
            Some((obj, tag)) => Ok(emit(schema, obj, r, tag)?.into()),
            None => plain(r),
        },
        (Type::Array { item, .. }, Kind::Array) => {
            let list = Array::new();
            for each in r.elements() {
                list.push(&value(schema, item, &each)?);
            }
            Ok(list.into())
        }
        // A 64-bit integer is a bigint. A `number` cannot hold it, which is the
        // whole reason this binding reads the bytes itself.
        (Type::Int(i), Kind::Int) if i.width == IntWidth::W64 => match r.as_int() {
            Some(Int::Signed(n)) => Ok(JsBigInt::from(n).into()),
            Some(Int::Unsigned(n)) => Ok(JsBigInt::from(n).into()),
            None => plain(r),
        },
        _ => plain(r),
    }
}

fn plain(r: &JsonRef<'_, '_>) -> Result<JsValue, JsValue> {
    Ok(match r.kind() {
        Kind::Null | Kind::Foreign | Kind::UnsafeInteger | Kind::IntegerTooWide => JsValue::NULL,
        Kind::Bool => JsValue::from_bool(r.as_bool().unwrap_or_default()),
        Kind::Int => match r.as_int() {
            // Safe as a number, or it would not have validated as a narrow
            // type; a 64-bit field took the bigint branch above.
            Some(n) => JsValue::from_f64(n.as_i128() as f64),
            None => JsValue::NULL,
        },
        Kind::Float => JsValue::from_f64(r.as_f64().unwrap_or_default()),
        Kind::String => JsValue::from_str(&r.as_str().unwrap_or_default()),
        Kind::Array => {
            let list = Array::new();
            for each in r.elements() {
                list.push(&plain(&each)?);
            }
            list.into()
        }
        Kind::Object => {
            let out = Object::new();
            for (key, each) in r.entries() {
                Reflect::set(&out, &JsValue::from_str(key.as_ref()), &plain(&each)?)?;
            }
            out.into()
        }
    })
}
