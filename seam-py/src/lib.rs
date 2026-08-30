//! PyO3 binding. Translation only: lower a Python object into a
//! `seam_core::Value`, hand it to the engine, raise or return. No rule logic
//! belongs in this file.

use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyTypeError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple};

use std::sync::Arc;

use seam_core::error::Segment;
use seam_core::schema::{ObjectType, Rule, Type};
use seam_core::value::Int;
use seam_core::{Code, Path, Value};

create_exception!(_seam, ValidationError, PyException);
create_exception!(_seam, ParseError, PyException);

#[pyclass(frozen, get_all, skip_from_py_object, module = "seam")]
#[derive(Clone)]
pub struct Issue {
    pub path: String,
    pub code: String,
    pub message: String,
}

#[pymethods]
impl Issue {
    fn __repr__(&self) -> String {
        format!(
            "Issue(path={:?}, code={:?}, message={:?})",
            self.path, self.code, self.message
        )
    }

    fn __str__(&self) -> String {
        format!("{}: {} ({})", self.path, self.message, self.code)
    }
}

#[pyclass(frozen, skip_from_py_object, module = "seam")]
#[derive(Clone, Copy)]
pub struct Limits {
    inner: seam_core::Limits,
}

#[pymethods]
impl Limits {
    #[new]
    #[pyo3(signature = (max_depth=None, max_items=None, max_string_bytes=None, max_object_keys=None))]
    fn new(
        max_depth: Option<usize>,
        max_items: Option<usize>,
        max_string_bytes: Option<usize>,
        max_object_keys: Option<usize>,
    ) -> Self {
        let d = seam_core::Limits::DEFAULT;
        Limits {
            inner: seam_core::Limits {
                max_depth: max_depth.unwrap_or(d.max_depth),
                max_items: max_items.unwrap_or(d.max_items),
                max_string_bytes: max_string_bytes.unwrap_or(d.max_string_bytes),
                max_object_keys: max_object_keys.unwrap_or(d.max_object_keys),
            },
        }
    }

    #[getter]
    fn max_depth(&self) -> usize {
        self.inner.max_depth
    }

    #[getter]
    fn max_items(&self) -> usize {
        self.inner.max_items
    }

    #[getter]
    fn max_string_bytes(&self) -> usize {
        self.inner.max_string_bytes
    }

    #[getter]
    fn max_object_keys(&self) -> usize {
        self.inner.max_object_keys
    }
}

#[pyclass(module = "seam")]
pub struct Schema {
    // Shared with every Validator bound to it, so binding one costs a refcount
    // rather than a copy of the schema.
    inner: Arc<seam_core::Schema>,
}

#[pymethods]
impl Schema {
    #[staticmethod]
    fn parse(source: &str) -> PyResult<Schema> {
        match seam_core::parse(source) {
            Ok(inner) => Ok(Schema { inner: Arc::new(inner) }),
            Err(e) => Err(ParseError::new_err(e.to_string())),
        }
    }

    #[staticmethod]
    fn load(path: std::path::PathBuf) -> PyResult<Schema> {
        let source = std::fs::read_to_string(&path)?;
        match seam_core::parse(&source) {
            Ok(inner) => Ok(Schema { inner: Arc::new(inner) }),
            Err(e) => Err(ParseError::new_err(format!("{}: {e}", path.display()))),
        }
    }

    fn type_names(&self) -> Vec<String> {
        self.inner.types.keys().cloned().collect()
    }

    /// The schema as plain data, for tooling that generates types.
    ///
    /// Deliberately not a validator: it carries shape, not rules-as-behaviour.
    /// Each binding renders its own language's types from this, which is why
    /// it is exposed rather than the code generator being written in Rust.
    fn describe<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let out = PyDict::new(py);
        for (name, ty) in &self.inner.types {
            out.set_item(name, describe_object(py, ty)?)?;
        }
        Ok(out)
    }

    /// Binds one type for repeated validation.
    ///
    /// Everything that does not depend on the payload is resolved here instead
    /// of on every call: the type lookup, the `datetime` classes, the limits.
    /// Prefer this wherever the same type is validated more than once.
    #[pyo3(signature = (type_name, limits=None))]
    fn validator(
        &self,
        py: Python<'_>,
        type_name: &str,
        limits: Option<&Limits>,
    ) -> PyResult<Validator> {
        if self.inner.get(type_name).is_none() {
            return Err(ValidationError::new_err(format!(
                "schema declares no type named `{type_name}`"
            )));
        }
        let datetime = py.import("datetime")?;
        Ok(Validator {
            schema: Arc::clone(&self.inner),
            type_name: type_name.to_string(),
            date_cls: datetime.getattr("date")?.unbind(),
            datetime_cls: datetime.getattr("datetime")?.unbind(),
            limits: limits.map_or(seam_core::Limits::DEFAULT, |l| l.inner),
        })
    }

    /// Convenience for one-off validation.
    ///
    /// Builds a [`Validator`] and discards it, so it pays the binding cost on
    /// every call. In a loop, bind once with `validator()` instead.
    #[pyo3(signature = (type_name, payload, limits=None))]
    fn validate<'py>(
        &self,
        type_name: &str,
        payload: &Bound<'py, PyAny>,
        limits: Option<&Limits>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.validator(payload.py(), type_name, limits)?
            .run(payload)
    }

    fn __repr__(&self) -> String {
        format!("Schema(types={:?})", self.type_names())
    }
}

/// One type of one schema, ready to validate.
#[pyclass(module = "seam")]
pub struct Validator {
    schema: Arc<seam_core::Schema>,
    type_name: String,
    // Resolved once. `Py<T>` is immutable here, so sharing a validator across
    // threads carries no synchronisation of its own.
    date_cls: Py<PyAny>,
    datetime_cls: Py<PyAny>,
    limits: seam_core::Limits,
}

#[pymethods]
impl Validator {
    fn __call__<'py>(&self, payload: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        self.run(payload)
    }

    fn validate<'py>(&self, payload: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        self.run(payload)
    }

    #[getter]
    fn type_name(&self) -> &str {
        &self.type_name
    }

    fn __repr__(&self) -> String {
        format!("Validator({})", self.type_name)
    }
}

impl Validator {
    fn run<'py>(&self, payload: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let py = payload.py();
        let ctx = Ctx {
            py,
            // A refcount bump, not an import: the classes were resolved when
            // the validator was bound.
            date_cls: self.date_cls.bind(py).clone(),
            datetime_cls: self.datetime_cls.bind(py).clone(),
        };

        let Some(object_type) = self.schema.get(&self.type_name) else {
            return Err(ValidationError::new_err(format!(
                "schema declares no type named `{}`",
                self.type_name
            )));
        };

        // Lowering is part of the contract, not a harness detail: an integer
        // Python can hold but the model cannot is caught here, before the
        // engine ever sees it.
        let value = match ctx.lower(payload, &mut Vec::new()) {
            Ok(v) => v,
            Err(LowerErr::Py(e)) => return Err(e),
            Err(LowerErr::Issue(issue)) => return Err(raise(py, vec![issue])),
        };

        if let Err(e) = seam_core::validate(&self.schema, &self.type_name, &value, self.limits) {
            let issues = e
                .issues
                .into_iter()
                .map(|i| Issue {
                    path: i.path.render(),
                    code: i.code.as_str().to_string(),
                    message: i.message,
                })
                .collect();
            return Err(raise(py, issues));
        }

        ctx.object_to_python(&self.schema, object_type, &value)
    }
}

fn describe_object<'py>(py: Python<'py>, ty: &ObjectType) -> PyResult<Bound<'py, PyDict>> {
    let fields = PyList::empty(py);
    for field in &ty.fields {
        let f = PyDict::new(py);
        f.set_item("name", &field.name)?;
        f.set_item("type", describe_type(py, &field.ty)?)?;
        f.set_item("optional", field.presence.optional)?;
        f.set_item("nullable", field.presence.nullable)?;

        let rules = PyList::empty(py);
        for rule in &field.rules {
            let r = PyDict::new(py);
            match rule {
                Rule::MinLen(n) => {
                    r.set_item("rule", "min_len")?;
                    r.set_item("value", n)?;
                }
                Rule::MaxLen(n) => {
                    r.set_item("rule", "max_len")?;
                    r.set_item("value", n)?;
                }
                Rule::MinItems(n) => {
                    r.set_item("rule", "min_items")?;
                    r.set_item("value", n)?;
                }
                Rule::MaxItems(n) => {
                    r.set_item("rule", "max_items")?;
                    r.set_item("value", n)?;
                }
                Rule::Range { min, max } => {
                    r.set_item("rule", "range")?;
                    r.set_item("min", min)?;
                    r.set_item("max", max)?;
                }
            }
            rules.append(r)?;
        }
        f.set_item("rules", rules)?;
        fields.append(f)?;
    }

    let out = PyDict::new(py);
    out.set_item("name", &ty.name)?;
    out.set_item("deny_unknown_fields", ty.deny_unknown_fields)?;
    out.set_item("fields", fields)?;
    Ok(out)
}

fn describe_type<'py>(py: Python<'py>, ty: &Type) -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(py);
    match ty {
        Type::Bool => {
            out.set_item("kind", "bool")?;
        }
        Type::Float => {
            out.set_item("kind", "float")?;
        }
        Type::String => {
            out.set_item("kind", "string")?;
        }
        Type::Date => {
            out.set_item("kind", "date")?;
        }
        Type::DateTime => {
            out.set_item("kind", "datetime")?;
        }
        Type::Int(int_ty) => {
            out.set_item("kind", "int")?;
            out.set_item("name", int_ty.name())?;
            out.set_item("signed", int_ty.signed)?;
            // What the JS binding needs to choose `bigint` over `number`, and
            // what any language needs to know it must range-check on the way
            // out. Kept here so no binding has to re-derive it.
            out.set_item("fits_js_number", int_ty.fits_js_number())?;
        }
        Type::Enum(values) => {
            out.set_item("kind", "enum")?;
            out.set_item("values", values.clone())?;
        }
        Type::Array { item, item_nullable } => {
            out.set_item("kind", "array")?;
            out.set_item("item", describe_type(py, item)?)?;
            out.set_item("item_nullable", item_nullable)?;
        }
        Type::Object(obj) => {
            out.set_item("kind", "object")?;
            out.set_item("object", describe_object(py, obj)?)?;
        }
        Type::Ref(name) => {
            out.set_item("kind", "ref")?;
            out.set_item("name", name)?;
        }
    }
    Ok(out)
}

fn raise(py: Python<'_>, issues: Vec<Issue>) -> PyErr {
    let summary = match issues.split_first() {
        None => "validation failed".to_string(),
        Some((first, [])) => format!("{}: {} ({})", first.path, first.message, first.code),
        Some((first, rest)) => format!(
            "{}: {} ({}), and {} more",
            first.path,
            first.message,
            first.code,
            rest.len()
        ),
    };

    let err = ValidationError::new_err(summary);
    // Attaching the structured issues is best effort: if it fails the caller
    // still gets a correct exception, just without the extra attributes.
    let list = PyList::empty(py);
    for issue in &issues {
        let _ = list.append(issue.clone());
    }
    let value = err.value(py);
    let _ = value.setattr("issues", list);
    if let Some(first) = issues.first() {
        let _ = value.setattr("path", &first.path);
        let _ = value.setattr("code", &first.code);
        let _ = value.setattr("message", &first.message);
    }
    err
}

enum LowerErr {
    Py(PyErr),
    Issue(Issue),
}

impl From<PyErr> for LowerErr {
    fn from(e: PyErr) -> Self {
        LowerErr::Py(e)
    }
}

/// Holds the `datetime` classes so they are imported once per call rather than
/// once per value.
struct Ctx<'py> {
    py: Python<'py>,
    date_cls: Bound<'py, PyAny>,
    datetime_cls: Bound<'py, PyAny>,
}

impl<'py> Ctx<'py> {
    fn lower(&self, ob: &Bound<'py, PyAny>, path: &mut Vec<Segment>) -> Result<Value, LowerErr> {
        if ob.is_none() {
            return Ok(Value::Null);
        }
        // Before int: in Python `bool` is a subclass of `int`, so checking int
        // first would turn every True into 1.
        if ob.is_instance_of::<PyBool>() {
            return Ok(Value::Bool(ob.extract::<bool>()?));
        }
        if ob.is_instance_of::<PyInt>() {
            return self.lower_int(ob, path);
        }
        if ob.is_instance_of::<PyFloat>() {
            return Ok(Value::Float(ob.extract::<f64>()?));
        }
        if ob.is_instance_of::<PyString>() {
            return Ok(Value::String(ob.extract::<String>()?));
        }
        // Before date: `datetime` is a subclass of `date`, so checking date
        // first would truncate every timestamp to a calendar day.
        if ob.is_instance(&self.datetime_cls)? {
            let iso: String = ob.call_method0("isoformat")?.extract()?;
            return Ok(Value::String(iso));
        }
        if ob.is_instance(&self.date_cls)? {
            let iso: String = ob.call_method0("isoformat")?.extract()?;
            return Ok(Value::String(iso));
        }
        if ob.is_instance_of::<PyList>() || ob.is_instance_of::<PyTuple>() {
            let mut out = Vec::new();
            for (i, item) in ob.try_iter()?.enumerate() {
                path.push(Segment::Index(i));
                let lowered = self.lower(&item?, path);
                path.pop();
                out.push(lowered?);
            }
            return Ok(Value::Array(out));
        }
        if let Ok(dict) = ob.cast::<PyDict>() {
            let mut out = std::collections::BTreeMap::new();
            for (k, v) in dict.iter() {
                let key: String = k.extract().map_err(|_| {
                    PyErr::new::<PyTypeError, _>("object keys must be strings".to_string())
                })?;
                path.push(Segment::Key(key.clone()));
                let lowered = self.lower(&v, path);
                path.pop();
                out.insert(key, lowered?);
            }
            return Ok(Value::Object(out));
        }

        let name = ob.get_type().name()?;
        Err(LowerErr::Py(PyErr::new::<PyTypeError, _>(format!(
            "{} cannot cross a Seam boundary at `{}`; \
             payloads hold null, bool, int, float, str, date, datetime, list and dict",
            name,
            Path(path.clone()).render()
        ))))
    }

    fn lower_int(&self, ob: &Bound<'py, PyAny>, path: &mut [Segment]) -> Result<Value, LowerErr> {
        if let Ok(n) = ob.extract::<i64>() {
            return Ok(Value::Int(Int::Signed(n)));
        }
        if let Ok(n) = ob.extract::<u64>() {
            return Ok(Value::Int(Int::Unsigned(n)));
        }
        // Python integers are arbitrary precision; the model stops at 64 bits.
        // Truncating here would be the exact bug Seam exists to prevent.
        Err(LowerErr::Issue(Issue {
            path: Path(path.to_vec()).render(),
            code: Code::IntegerTooWide.as_str().to_string(),
            message: "integer is wider than 64 bits".to_string(),
        }))
    }

    fn object_to_python(
        &self,
        schema: &seam_core::Schema,
        ty: &ObjectType,
        value: &Value,
    ) -> PyResult<Bound<'py, PyAny>> {
        let out = PyDict::new(self.py);
        let Value::Object(map) = value else {
            return Ok(out.into_any());
        };

        for field in &ty.fields {
            // An absent key stays absent. That is the whole point: `"bio" in
            // result` is how a caller reads absence, and writing None here
            // would collapse it into null.
            if let Some(v) = map.get(&field.name) {
                out.set_item(&field.name, self.to_python(schema, &field.ty, v)?)?;
            }
        }

        if !ty.deny_unknown_fields {
            for (k, v) in map {
                if ty.field(k).is_none() {
                    out.set_item(k, self.untyped_to_python(v)?)?;
                }
            }
        }

        Ok(out.into_any())
    }

    fn to_python(
        &self,
        schema: &seam_core::Schema,
        ty: &Type,
        value: &Value,
    ) -> PyResult<Bound<'py, PyAny>> {
        if matches!(value, Value::Null) {
            return Ok(self.py.None().into_bound(self.py));
        }
        match (ty, value) {
            (Type::Date, Value::String(s)) => self.date_cls.call_method1("fromisoformat", (s,)),
            (Type::DateTime, Value::String(s)) => self
                .datetime_cls
                .call_method1("fromisoformat", (python_iso(s),)),
            (Type::Array { item, .. }, Value::Array(items)) => {
                let list = PyList::empty(self.py);
                for v in items {
                    list.append(self.to_python(schema, item, v)?)?;
                }
                Ok(list.into_any())
            }
            (Type::Object(obj), _) => self.object_to_python(schema, obj, value),
            (Type::Ref(name), _) => match schema.get(name) {
                Some(obj) => self.object_to_python(schema, obj, value),
                None => self.untyped_to_python(value),
            },
            _ => self.untyped_to_python(value),
        }
    }

    fn untyped_to_python(&self, value: &Value) -> PyResult<Bound<'py, PyAny>> {
        Ok(match value {
            Value::Null => self.py.None().into_bound(self.py),
            Value::Bool(b) => b.into_pyobject(self.py)?.to_owned().into_any(),
            Value::Int(Int::Signed(n)) => n.into_pyobject(self.py)?.into_any(),
            Value::Int(Int::Unsigned(n)) => n.into_pyobject(self.py)?.into_any(),
            Value::Float(f) => f.into_pyobject(self.py)?.into_any(),
            Value::String(s) => s.into_pyobject(self.py)?.into_any(),
            Value::Array(items) => {
                let list = PyList::empty(self.py);
                for v in items {
                    list.append(self.untyped_to_python(v)?)?;
                }
                list.into_any()
            }
            Value::Object(map) => {
                let d = PyDict::new(self.py);
                for (k, v) in map {
                    d.set_item(k, self.untyped_to_python(v)?)?;
                }
                d.into_any()
            }
        })
    }
}

/// `datetime.fromisoformat` only became lenient in 3.11: before that it wants
/// an explicit offset and exactly three or six fractional digits. The core has
/// already validated the shape, so this only reshapes it.
fn python_iso(s: &str) -> String {
    let (body, offset) = match s.as_bytes().last() {
        Some(b'Z' | b'z') => (&s[..s.len() - 1], "+00:00".to_string()),
        _ => match s.get(19..).and_then(|tail| {
            tail.find(['+', '-'])
                .map(|i| (&s[..19 + i], s[19 + i..].to_string()))
        }) {
            Some((body, off)) => (body, off),
            None => (s, String::new()),
        },
    };

    let (base, frac) = match body.split_once('.') {
        Some((b, f)) => (b, Some(f)),
        None => (body, None),
    };

    let mut out = String::from(base);
    if let Some(f) = frac {
        let mut micros: String = f.chars().filter(char::is_ascii_digit).take(6).collect();
        while micros.len() < 6 {
            micros.push('0');
        }
        out.push('.');
        out.push_str(&micros);
    }
    out.push_str(&offset);
    out
}

#[pymodule]
fn _seam(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    m.add_class::<Schema>()?;
    m.add_class::<Validator>()?;
    m.add_class::<Issue>()?;
    m.add_class::<Limits>()?;
    m.add("ValidationError", py.get_type::<ValidationError>())?;
    m.add("ParseError", py.get_type::<ParseError>())?;
    m.add("__version__", seam_core::VERSION)?;
    Ok(())
}
