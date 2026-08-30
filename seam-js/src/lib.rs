//! Node binding. Translation only: read the host's values, hand them to the
//! engine, return or report. No rule logic belongs in this file.
//!
//! Three things are specific to JavaScript and all three are decided here:
//! `undefined` means absent, a 64-bit integer is a `bigint`, and a `number`
//! past 2^53 is refused rather than validated as the wrong value.

use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::borrow::Cow;

use seam_core::input::{Input, Kind};
use seam_core::json::Ref as JsonRef;
use seam_core::schema::{IntWidth, ObjectType, Rule, Type};
use seam_core::value::{Int, Slot};

/// Beyond this a JavaScript `number` no longer holds every integer, so a value
/// above it has already lost information before Seam is called.
const MAX_SAFE: f64 = 9_007_199_254_740_991.0;

#[napi(object)]
pub struct JsIssue {
    pub path: String,
    pub code: String,
    pub message: String,
}

#[napi(object)]
pub struct JsLimits {
    pub max_depth: Option<u32>,
    pub max_items: Option<u32>,
    pub max_string_bytes: Option<u32>,
    pub max_object_keys: Option<u32>,
}

impl JsLimits {
    fn to_core(&self) -> seam_core::Limits {
        let d = seam_core::Limits::DEFAULT;
        seam_core::Limits {
            max_depth: self.max_depth.map_or(d.max_depth, |v| v as usize),
            max_items: self.max_items.map_or(d.max_items, |v| v as usize),
            max_string_bytes: self
                .max_string_bytes
                .map_or(d.max_string_bytes, |v| v as usize),
            max_object_keys: self
                .max_object_keys
                .map_or(d.max_object_keys, |v| v as usize),
        }
    }
}

#[napi]
pub struct Schema {
    inner: std::sync::Arc<seam_core::Schema>,
}

#[napi]
impl Schema {
    #[napi(factory)]
    pub fn parse(source: String) -> Result<Schema> {
        match seam_core::parse(&source) {
            Ok(inner) => Ok(Schema { inner: std::sync::Arc::new(inner) }),
            Err(e) => Err(Error::new(Status::InvalidArg, e.to_string())),
        }
    }

    #[napi(factory)]
    pub fn load(path: String) -> Result<Schema> {
        let source = std::fs::read_to_string(&path)
            .map_err(|e| Error::new(Status::GenericFailure, format!("{path}: {e}")))?;
        Schema::parse(source)
    }

    #[napi]
    pub fn type_names(&self) -> Vec<String> {
        self.inner.types.keys().cloned().collect()
    }

    /// The schema as plain data, for tooling that generates types.
    ///
    /// Deliberately not a validator: it carries shape, not rules-as-behaviour.
    /// Each binding renders its own language's types from this, which is why
    /// it is exposed rather than the code generator being written in Rust.
    /// The keys are camelCase here and snake_case in Python for the same
    /// reason: the data is crossing into a language, not out of one.
    #[napi]
    pub fn describe<'env>(&self, env: &'env Env) -> Result<Object<'env>> {
        let mut out = Object::new(env)?;
        for (name, ty) in &self.inner.types {
            out.set(name.as_str(), describe_object(env, ty)?)?;
        }
        Ok(out)
    }

    /// Binds one type. Everything that does not depend on the payload is
    /// resolved here rather than on every call.
    #[napi]
    pub fn validator(&self, type_name: String, limits: Option<JsLimits>) -> Result<Validator> {
        if self.inner.get(&type_name).is_none() {
            return Err(Error::new(
                Status::InvalidArg,
                format!("schema declares no type named `{type_name}`"),
            ));
        }
        Ok(Validator {
            schema: std::sync::Arc::clone(&self.inner),
            type_name,
            limits: limits.map_or(seam_core::Limits::DEFAULT, |l| l.to_core()),
        })
    }
}

#[napi]
pub struct Validator {
    schema: std::sync::Arc<seam_core::Schema>,
    type_name: String,
    limits: seam_core::Limits,
}

#[napi]
impl Validator {
    #[napi(getter)]
    pub fn type_name(&self) -> String {
        self.type_name.clone()
    }

    #[napi]
    pub fn validate<'env>(&self, env: &'env Env, payload: Unknown<'env>) -> Result<Object<'env>> {
        let object_type = self.schema.get(&self.type_name).ok_or_else(|| {
            Error::new(
                Status::InvalidArg,
                format!("schema declares no type named `{}`", self.type_name),
            )
        })?;

        // Raw JSON is read by Seam rather than by the host: `JSON.parse` would
        // already have flattened a 64-bit integer into a double, and nothing
        // afterwards brings those bits back.
        if let Some(bytes) = as_json_bytes(&payload)? {
            let doc = seam_core::json::Document::parse(&bytes, self.limits)
                .map_err(|e| Error::new(Status::InvalidArg, e.to_string()))?;
            let root = doc.root();
            return match seam_core::validate(&self.schema, &self.type_name, &root, self.limits) {
                Err(e) => refused(env, e),
                Ok(()) => {
                    let value = emit_json(env, &self.schema, object_type, &root)?;
                    accepted(env, value.to_unknown())
                }
            };
        }

        let input = JsInput { env, value: payload, kind: classify(&payload) };
        match seam_core::validate(&self.schema, &self.type_name, &input, self.limits) {
            Err(e) => refused(env, e),
            Ok(()) => {
                let value = emit_js(env, &self.schema, object_type, &input)?;
                accepted(env, value.to_unknown())
            }
        }
    }
}

/// What a validation produced: the value, or the reasons it was refused.
///
/// The idiomatic surface lives in `index.js`, which turns this into a returned
/// value or a thrown `SeamValidationError`. Deciding on the JavaScript side is
/// what makes the error a real `Error` with a real stack.
fn accepted<'env>(env: &'env Env, value: Unknown<'env>) -> Result<Object<'env>> {
    let mut out = Object::new(env)?;
    out.set("ok", true)?;
    out.set("value", value)?;
    Ok(out)
}

fn refused<'env>(env: &'env Env, e: seam_core::ValidationError) -> Result<Object<'env>> {
    let issues: Vec<JsIssue> = e
        .issues
        .into_iter()
        .map(|i| JsIssue {
            path: i.path.render(),
            code: i.code.as_str().to_string(),
            message: i.message,
        })
        .collect();
    let mut out = Object::new(env)?;
    out.set("ok", false)?;
    out.set("issues", issues)?;
    Ok(out)
}

/// `Buffer`, `Uint8Array` and `string` are JSON; anything else is a value.
fn as_json_bytes(value: &Unknown<'_>) -> Result<Option<Vec<u8>>> {
    match value.get_type()? {
        ValueType::String => Ok(Some(
            value.coerce_to_string()?.into_utf8()?.as_str()?.into(),
        )),
        ValueType::Object => {
            if let Ok(buf) = Uint8Array::from_unknown(*value) {
                return Ok(Some(buf.to_vec()));
            }
            Ok(None)
        }
        _ => Ok(None),
    }
}

// --------------------------------------------------------- describing

fn describe_object<'env>(env: &'env Env, ty: &ObjectType) -> Result<Object<'env>> {
    let mut fields = env.create_array(ty.fields.len() as u32)?;
    for (i, field) in ty.fields.iter().enumerate() {
        let mut f = Object::new(env)?;
        f.set("name", field.name.as_str())?;
        f.set("type", describe_type(env, &field.ty)?)?;
        // Two keys, not one, because absence and nullability are two axes.
        f.set("optional", field.presence.optional)?;
        f.set("nullable", field.presence.nullable)?;
        f.set("rules", describe_rules(env, &field.rules)?)?;
        fields.set(i as u32, f)?;
    }

    let mut out = Object::new(env)?;
    out.set("name", ty.name.as_str())?;
    out.set("denyUnknownFields", ty.deny_unknown_fields)?;
    // Declaration order, not sorted: it is the order errors are reported in,
    // and the order the generated type should read in.
    out.set("fields", fields.coerce_to_object()?)?;
    Ok(out)
}

fn describe_rules<'env>(env: &'env Env, rules: &[Rule]) -> Result<Object<'env>> {
    let mut out = env.create_array(rules.len() as u32)?;
    for (i, rule) in rules.iter().enumerate() {
        let mut r = Object::new(env)?;
        // The rule names are the `.seam` spelling, so they stay snake_case:
        // they are values from the schema language, not JavaScript keys.
        match rule {
            Rule::MinLen(n) => {
                r.set("rule", "min_len")?;
                r.set("value", *n as f64)?;
            }
            Rule::MaxLen(n) => {
                r.set("rule", "max_len")?;
                r.set("value", *n as f64)?;
            }
            Rule::MinItems(n) => {
                r.set("rule", "min_items")?;
                r.set("value", *n as f64)?;
            }
            Rule::MaxItems(n) => {
                r.set("rule", "max_items")?;
                r.set("value", *n as f64)?;
            }
            Rule::Range { min, max } => {
                r.set("rule", "range")?;
                // A `u64` bound does not fit a `number`, and a bound that is
                // not the bound would be worse than no bound at all.
                r.set("min", BigInt::from(*min))?;
                r.set("max", BigInt::from(*max))?;
            }
        }
        out.set(i as u32, r)?;
    }
    out.coerce_to_object()
}

fn describe_type<'env>(env: &'env Env, ty: &Type) -> Result<Object<'env>> {
    let mut out = Object::new(env)?;
    match ty {
        Type::Bool => {
            out.set("kind", "bool")?;
        }
        Type::Float => {
            out.set("kind", "float")?;
        }
        Type::String => {
            out.set("kind", "string")?;
        }
        Type::Date => {
            out.set("kind", "date")?;
        }
        Type::DateTime => {
            out.set("kind", "datetime")?;
        }
        Type::Int(int_ty) => {
            out.set("kind", "int")?;
            out.set("name", int_ty.name())?;
            out.set("signed", int_ty.signed)?;
            // What tells a generator to write `bigint` rather than `number`.
            // Decided in the core so no binding re-derives the rule.
            out.set("fitsJsNumber", int_ty.fits_js_number())?;
        }
        Type::Enum(values) => {
            out.set("kind", "enum")?;
            let mut list = env.create_array(values.len() as u32)?;
            for (i, v) in values.iter().enumerate() {
                list.set(i as u32, v.as_str())?;
            }
            out.set("values", list.coerce_to_object()?)?;
        }
        Type::Array { item, item_nullable } => {
            out.set("kind", "array")?;
            out.set("item", describe_type(env, item)?)?;
            // An element has two states, a value or null. Absence is a
            // property of a key, and an array has no keys.
            out.set("itemNullable", *item_nullable)?;
        }
        Type::Object(obj) => {
            out.set("kind", "object")?;
            out.set("object", describe_object(env, obj)?)?;
        }
        Type::Ref(name) => {
            out.set("kind", "ref")?;
            out.set("name", name.as_str())?;
        }
    }
    Ok(out)
}

// ------------------------------------------------------------- reading JS

struct JsInput<'env> {
    env: &'env Env,
    value: Unknown<'env>,
    kind: Kind,
}

impl<'env> JsInput<'env> {
    fn child(&self, value: Unknown<'env>) -> JsInput<'env> {
        JsInput { env: self.env, kind: classify(&value), value }
    }

    fn object(&self) -> Option<Object<'env>> {
        Object::from_unknown(self.value).ok()
    }
}

fn classify(value: &Unknown<'_>) -> Kind {
    match value.get_type() {
        Ok(ValueType::Null | ValueType::Undefined) => Kind::Null,
        Ok(ValueType::Boolean) => Kind::Bool,
        Ok(ValueType::String) => Kind::String,
        Ok(ValueType::BigInt) => Kind::Int,
        Ok(ValueType::Number) => {
            let Ok(n) = f64::from_unknown(*value) else {
                return Kind::Foreign;
            };
            if n.fract() != 0.0 || !n.is_finite() {
                return Kind::Float;
            }
            // Past 2^53 a `number` no longer holds every integer, so this one
            // is already not the value that was sent.
            if n.abs() > MAX_SAFE {
                return Kind::UnsafeInteger;
            }
            Kind::Int
        }
        Ok(ValueType::Object) => {
            match Object::from_unknown(*value) {
                Ok(o) => {
                    if o.is_array().unwrap_or(false) {
                        Kind::Array
                    } else if is_date(&o) {
                        // A date reaches the engine in its wire form, so the
                        // same rules apply as to the string JSON would carry.
                        Kind::String
                    } else {
                        Kind::Object
                    }
                }
                Err(_) => Kind::Foreign,
            }
        }
        _ => Kind::Foreign,
    }
}

fn is_date(o: &Object<'_>) -> bool {
    o.get_named_property::<Unknown>("toISOString")
        .map(|f| matches!(f.get_type(), Ok(ValueType::Function)))
        .unwrap_or(false)
}

impl<'env> Input for JsInput<'env> {
    // Tied to the environment rather than to the borrow: a child outlives the
    // call that produced it, which is what lets the emitter reuse it.
    type Child<'x>
        = JsInput<'env>
    where
        Self: 'x;

    fn kind(&self) -> Kind {
        self.kind
    }

    fn as_bool(&self) -> Option<bool> {
        bool::from_unknown(self.value).ok()
    }

    fn as_int(&self) -> Option<Int> {
        match self.value.get_type().ok()? {
            ValueType::BigInt => {
                let big = BigInt::from_unknown(self.value).ok()?;
                let (signed, value, lossless) = big.get_u128();
                if !lossless {
                    return None;
                }
                if signed {
                    i64::try_from(value).ok().map(|v| Int::Signed(-v))
                } else {
                    u64::try_from(value).ok().map(Int::from)
                }
            }
            ValueType::Number => {
                let n = f64::from_unknown(self.value).ok()?;
                if n.abs() > MAX_SAFE {
                    return None;
                }
                Some(Int::Signed(n as i64))
            }
            _ => None,
        }
    }

    fn as_f64(&self) -> Option<f64> {
        f64::from_unknown(self.value).ok()
    }

    fn as_str(&self) -> Option<Cow<'_, str>> {
        if let Ok(ValueType::String) = self.value.get_type() {
            let s = self.value.coerce_to_string().ok()?;
            return Some(Cow::Owned(s.into_utf8().ok()?.as_str().ok()?.to_string()));
        }
        // A Date: hand the engine the wire form and let its rules decide.
        // Applied to the object, not called bare: `toISOString` reads `this`.
        let o = self.object()?;
        let iso: String = o
            .get_named_property::<Function<(), String>>("toISOString")
            .ok()?
            .apply(o, ())
            .ok()?;
        Some(Cow::Owned(iso))
    }

    fn len(&self) -> usize {
        match self.kind {
            Kind::Array => self
                .object()
                .and_then(|o| o.get_array_length().ok())
                .unwrap_or(0) as usize,
            Kind::Object => self
                .object()
                .and_then(|o| o.get_property_names().ok())
                .and_then(|n| n.get_array_length().ok())
                .unwrap_or(0) as usize,
            _ => 0,
        }
    }

    fn item(&self, index: usize) -> Option<Self::Child<'_>> {
        if self.kind != Kind::Array {
            return None;
        }
        let o = self.object()?;
        let v: Unknown = o.get_element(index as u32).ok()?;
        Some(self.child(v))
    }

    fn slot(&self, key: &str) -> Slot<Self::Child<'_>> {
        if self.kind != Kind::Object {
            return Slot::Absent;
        }
        let Some(o) = self.object() else {
            return Slot::Absent;
        };
        let Ok(found) = o.get_named_property::<Unknown>(key) else {
            return Slot::Absent;
        };
        match found.get_type() {
            // `undefined` is how JavaScript spells a key that was not sent,
            // and `JSON.stringify` drops it, so absent is the honest reading.
            Ok(ValueType::Undefined) => Slot::Absent,
            Ok(ValueType::Null) => Slot::Null,
            Ok(_) => Slot::Present(self.child(found)),
            Err(_) => Slot::Absent,
        }
    }

    fn each_key(&self, f: &mut dyn FnMut(&str)) {
        let Some(o) = self.object() else { return };
        let Ok(names) = o.get_property_names() else {
            return;
        };
        let len = names.get_array_length().unwrap_or(0);
        for i in 0..len {
            if let Ok(name) = names.get_element::<String>(i) {
                f(&name);
            }
        }
    }
}

// -------------------------------------------------------------- writing JS

fn emit_js<'env>(
    env: &'env Env,
    schema: &seam_core::Schema,
    ty: &ObjectType,
    input: &JsInput<'env>,
) -> Result<Object<'env>> {
    let mut out = Object::new(env)?;
    for field in &ty.fields {
        // Absent stays absent: the property is simply not created, which is
        // what `"bio" in user` and `JSON.stringify` both read as not sent.
        let value = match input.slot(&field.name) {
            Slot::Absent => continue,
            Slot::Null => {
                out.set(field.name.as_str(), Null)?;
                continue;
            }
            Slot::Present(v) => v,
        };
        set_js(env, schema, &mut out, &field.name, &field.ty, &value)?;
    }
    Ok(out)
}

fn set_js<'env>(
    env: &'env Env,
    schema: &seam_core::Schema,
    out: &mut Object<'env>,
    key: &str,
    ty: &Type,
    value: &JsInput<'env>,
) -> Result<()> {
    match ty {
        Type::DateTime => {
            let text = value.as_str().unwrap_or_default().into_owned();
            let ctor: Function<String, Unknown> = env
                .get_global()?
                .get_named_property::<Function<String, Unknown>>("Date")?;
            out.set(key, ctor.new_instance(text)?)?;
        }
        Type::Object(obj) => {
            out.set(key, emit_js(env, schema, obj, value)?)?;
        }
        Type::Ref(name) => match schema.get(name) {
            Some(obj) => out.set(key, emit_js(env, schema, obj, value)?)?,
            None => out.set(key, value.value)?,
        },
        Type::Array { item, .. } if needs_work(item, schema) => {
            let mut list = env.create_array(value.len() as u32)?;
            for i in 0..value.len() {
                if let Some(child) = value.item(i) {
                    let mut holder = Object::new(env)?;
                    set_js(env, schema, &mut holder, "v", item, &child)?;
                    list.set(i as u32, holder.get_named_property::<Unknown>("v")?)?;
                }
            }
            out.set(key, list)?;
        }
        // A `Date` stays the ISO string it arrived as, and everything else is
        // handed back unchanged.
        _ => out.set(key, value.value)?,
    }
    Ok(())
}

/// Whether a type's values come back as something other than what arrived.
fn needs_work(ty: &Type, schema: &seam_core::Schema) -> bool {
    match ty {
        Type::DateTime | Type::Object(_) => true,
        Type::Int(i) => i.width == IntWidth::W64,
        Type::Array { item, .. } => needs_work(item, schema),
        Type::Ref(name) => schema.get(name).is_some(),
        _ => false,
    }
}

fn emit_json<'env>(
    env: &'env Env,
    schema: &seam_core::Schema,
    ty: &ObjectType,
    r: &JsonRef<'_, '_>,
) -> Result<Object<'env>> {
    let mut out = Object::new(env)?;
    for field in &ty.fields {
        match r.slot(&field.name) {
            Slot::Absent => continue,
            Slot::Null => out.set(field.name.as_str(), Null)?,
            Slot::Present(v) => {
                let value = json_value(env, schema, &field.ty, &v)?;
                out.set(field.name.as_str(), value)?;
            }
        }
    }
    Ok(out)
}

fn json_value<'env>(
    env: &'env Env,
    schema: &seam_core::Schema,
    ty: &Type,
    r: &JsonRef<'_, '_>,
) -> Result<Unknown<'env>> {
    match (ty, r.kind()) {
        (Type::DateTime, Kind::String) => {
            let text = r.as_str().unwrap_or_default().into_owned();
            let ctor = env
                .get_global()?
                .get_named_property::<Function<String, Unknown>>("Date")?;
            ctor.new_instance(text)
        }
        (Type::Object(obj), _) => Ok(emit_json(env, schema, obj, r)?.to_unknown()),
        (Type::Ref(name), _) => match schema.get(name) {
            Some(obj) => Ok(emit_json(env, schema, obj, r)?.to_unknown()),
            None => json_plain(env, r),
        },
        (Type::Array { item, .. }, Kind::Array) => {
            let mut list = env.create_array(r.len() as u32)?;
            for (i, each) in r.elements().enumerate() {
                list.set(i as u32, json_value(env, schema, item, &each)?)?;
            }
            Ok(list.coerce_to_object()?.to_unknown())
        }
        // A 64-bit integer is a bigint. A `number` cannot hold it, which is the
        // whole reason this binding exists.
        (Type::Int(i), Kind::Int) if i.width == IntWidth::W64 => match r.as_int() {
            Some(Int::Signed(n)) => Ok(BigInt::from(n).into_unknown(env)?),
            Some(Int::Unsigned(n)) => Ok(BigInt::from(n).into_unknown(env)?),
            None => json_plain(env, r),
        },
        _ => json_plain(env, r),
    }
}

fn json_plain<'env>(env: &'env Env, r: &JsonRef<'_, '_>) -> Result<Unknown<'env>> {
    match r.kind() {
        Kind::Null | Kind::Foreign | Kind::UnsafeInteger | Kind::IntegerTooWide => {
            Null.into_unknown(env)
        }
        Kind::Bool => r.as_bool().unwrap_or_default().into_unknown(env),
        Kind::Int => match r.as_int() {
            // Safe as a number, or it would not have validated as a narrow
            // type; a 64-bit field took the bigint branch above.
            Some(n) => (n.as_i128() as f64).into_unknown(env),
            None => Null.into_unknown(env),
        },
        Kind::Float => r.as_f64().unwrap_or_default().into_unknown(env),
        Kind::String => r
            .as_str()
            .unwrap_or_default()
            .into_owned()
            .into_unknown(env),
        Kind::Array => {
            let mut list = env.create_array(r.len() as u32)?;
            for (i, each) in r.elements().enumerate() {
                list.set(i as u32, json_plain(env, &each)?)?;
            }
            Ok(list.coerce_to_object()?.to_unknown())
        }
        Kind::Object => {
            let mut out = Object::new(env)?;
            for (key, each) in r.entries() {
                let value = json_plain(env, &each)?;
                out.set(key.as_ref(), value)?;
            }
            Ok(out.to_unknown())
        }
    }
}
