//! PyO3 binding. Translation only: lower a Python object into a
//! `seam_core::Value`, hand it to the engine, raise or return. No rule logic
//! belongs in this file.

use pyo3::create_exception;
use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::{
    PyBool, PyByteArray, PyBytes, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple, PyType,
};

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use seam_core::input::{Input, Kind};
use seam_core::schema::{ObjectType, Rule, Type};
use seam_core::value::{Int, Slot, Value};
use seam_core::Code;

create_exception!(_seam, ParseError, PyException);

/// What a `ValidationError` carries, with nothing built until it is read.
///
/// The exception itself is declared in Python and overrides no `__init__`, so
/// raising costs one allocation here and the interpreter's own C-level
/// bookkeeping. The path strings, the `Issue` objects and the summary are all
/// produced on first access. Rejecting a request is a hot path in any service
/// facing a network, and it should not cost more than accepting one.
#[pyclass(frozen, skip_from_py_object, module = "seam")]
pub struct Issues {
    inner: Vec<seam_core::Issue>,
}

#[pymethods]
impl Issues {
    fn list<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        let out = PyList::empty(py);
        for issue in &self.inner {
            out.append(Issue {
                path: issue.path.render(),
                code: issue.code.as_str().to_string(),
                message: issue.message.clone(),
            })?;
        }
        Ok(out)
    }

    fn summary(&self) -> String {
        match self.inner.split_first() {
            None => "validation failed".to_string(),
            Some((first, [])) => {
                format!(
                    "{}: {} ({})",
                    first.path.render(),
                    first.message,
                    first.code
                )
            }
            Some((first, rest)) => format!(
                "{}: {} ({}), and {} more",
                first.path.render(),
                first.message,
                first.code,
                rest.len()
            ),
        }
    }

    fn first_path(&self) -> String {
        self.inner
            .first()
            .map_or_else(String::new, |i| i.path.render())
    }

    fn first_code(&self) -> &str {
        self.inner.first().map_or("", |i| i.code.as_str())
    }

    fn first_message(&self) -> &str {
        self.inner.first().map_or("", |i| i.message.as_str())
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }
}

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
            return Err(raise(
                py,
                one_issue(
                    type_name,
                    Code::UnknownType,
                    format!("schema declares no type named `{type_name}`"),
                ),
            ));
        }
        let datetime = py.import("datetime")?;
        Ok(Validator {
            schema: Arc::clone(&self.inner),
            type_name: type_name.to_string(),
            bindings: Bindings {
                date: datetime.getattr("date")?.unbind(),
                datetime: datetime.getattr("datetime")?.unbind(),
                keys: Bindings::intern_keys(py, &self.inner),
            },
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
    // Resolved once. Immutable after binding, so sharing a validator across
    // threads carries no synchronisation of its own.
    bindings: Bindings,
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

        // Raw JSON is parsed here rather than by the host, because the rules
        // that keep an integer intact have to apply while the bytes are read.
        // A caller never has to know that.
        //
        // Checked by type, not by whether the value happens to extract: a list
        // of small integers extracts as `Vec<u8>` perfectly well.
        if let Ok(b) = payload.cast::<PyBytes>() {
            return self.run_json(py, b.as_bytes());
        }
        if let Ok(b) = payload.cast::<PyByteArray>() {
            // Mutable, so read a copy rather than borrow into it.
            return self.run_json(py, &b.to_vec());
        }
        if payload.is_instance_of::<PyString>() {
            let text = payload.extract::<String>()?;
            return self.run_json(py, text.as_bytes());
        }

        let Some(object_type) = self.schema.get(&self.type_name) else {
            return Err(raise(
                py,
                one_issue(
                    &self.type_name,
                    Code::UnknownType,
                    format!("schema declares no type named `{}`", self.type_name),
                ),
            ));
        };

        // Read in place. Nothing is copied before the rules run, so a rejected
        // payload is never materialised at all.
        let input = PyInput::new(payload.clone(), &self.bindings);

        if let Err(e) = seam_core::validate(&self.schema, &self.type_name, &input, self.limits) {
            return Err(raise(py, e.issues));
        }

        Out { py, schema: &self.schema, bindings: &self.bindings }.object(object_type, payload)
    }

    fn run_json<'py>(&self, py: Python<'py>, bytes: &[u8]) -> PyResult<Bound<'py, PyAny>> {
        let value = seam_core::json::parse(bytes, self.limits)
            .map_err(|e| ParseError::new_err(e.to_string()))?;

        if let Err(e) = seam_core::validate(&self.schema, &self.type_name, &value, self.limits) {
            return Err(raise(py, e.issues));
        }

        let Some(object_type) = self.schema.get(&self.type_name) else {
            return Err(raise(
                py,
                one_issue(
                    &self.type_name,
                    Code::UnknownType,
                    format!("schema declares no type named `{}`", self.type_name),
                ),
            ));
        };

        Out { py, schema: &self.schema, bindings: &self.bindings }
            .object_from_value(object_type, &value)
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

/// Builds the exception without touching Python beyond allocating it.
///
/// The issues stay on the Rust side until something reads them, so the raise
/// path costs one object instead of a list, an object per issue, and four
/// attribute writes.
/// The exception class, looked up once per interpreter.
///
/// Resolved lazily rather than at module init: `seam/__init__.py` imports this
/// extension, so reaching back for it eagerly would be a cycle.
///
/// `PyOnceLock` rather than a plain `OnceLock`: a cached Python object belongs
/// to the interpreter that created it, and this one keys on that instead of
/// handing a stale type to a subinterpreter.
static VALIDATION_ERROR: PyOnceLock<Py<PyType>> = PyOnceLock::new();

fn error_type<'py>(py: Python<'py>) -> PyResult<Bound<'py, PyType>> {
    let cls = VALIDATION_ERROR.get_or_try_init(py, || {
        py.import("seam")?
            .getattr("ValidationError")?
            .cast_into::<PyType>()
            .map(Bound::unbind)
            .map_err(PyErr::from)
    })?;
    Ok(cls.bind(py).clone())
}

fn raise(py: Python<'_>, issues: Vec<seam_core::Issue>) -> PyErr {
    let raw = match Py::new(py, Issues { inner: issues }) {
        Ok(raw) => raw,
        // Allocation failed, so the interpreter is in trouble; that error still
        // has to surface as something.
        Err(e) => return e,
    };
    match error_type(py) {
        // `from_type` stores the class and its argument without instantiating,
        // so even the exception object waits until something needs it.
        Ok(cls) => PyErr::from_type(cls, (raw,)),
        Err(e) => e,
    }
}

/// A single issue at a top-level key, for the failures the engine does not
/// produce itself.
fn one_issue(key: &str, code: Code, message: String) -> Vec<seam_core::Issue> {
    vec![seam_core::Issue {
        path: seam_core::Path(vec![seam_core::Segment::Key(key.to_string())]),
        code,
        message,
    }]
}

/// Everything resolved once when a validator is bound.
struct Bindings {
    date: Py<PyAny>,
    datetime: Py<PyAny>,
    /// One interned Python string per field name in the schema.
    ///
    /// Without this a field costs two fresh Python strings per call: one to
    /// look the key up in the payload and one to write it into the result.
    /// Interned strings also carry a precomputed hash, so the dict lookup that
    /// follows is cheaper too.
    keys: HashMap<String, Py<PyString>>,
}

impl Bindings {
    fn key<'py>(&self, py: Python<'py>, name: &str) -> Option<&Bound<'py, PyString>> {
        self.keys.get(name).map(|k| k.bind(py))
    }

    /// Walks every declared field name once, at bind time.
    fn intern_keys(py: Python<'_>, schema: &seam_core::Schema) -> HashMap<String, Py<PyString>> {
        let mut keys = HashMap::new();
        for ty in schema.types.values() {
            for field in &ty.fields {
                keys.entry(field.name.clone())
                    .or_insert_with(|| PyString::intern(py, &field.name).unbind());
            }
        }
        keys
    }
}

/// A Python object the engine reads in place.
///
/// No copy is made: `kind` is computed once on construction because the
/// validator asks for it more than once, and everything else reads through to
/// the object itself.
struct PyInput<'a, 'py> {
    ob: Bound<'py, PyAny>,
    bindings: &'a Bindings,
    kind: Kind,
}

impl<'a, 'py> PyInput<'a, 'py> {
    fn new(ob: Bound<'py, PyAny>, bindings: &'a Bindings) -> Self {
        let kind = classify(&ob, bindings);
        PyInput { ob, bindings, kind }
    }

    fn child(&self, ob: Bound<'py, PyAny>) -> PyInput<'a, 'py> {
        PyInput::new(ob, self.bindings)
    }
}

/// The order here is the whole correctness story of this file.
///
/// `bool` is a subclass of `int` in Python and `datetime` is a subclass of
/// `date`, so checking the general case first would silently turn every `True`
/// into `1` and every timestamp into a calendar day.
fn classify(ob: &Bound<'_, PyAny>, bindings: &Bindings) -> Kind {
    if ob.is_none() {
        return Kind::Null;
    }
    if ob.is_instance_of::<PyBool>() {
        return Kind::Bool;
    }
    if ob.is_instance_of::<PyInt>() {
        // Python integers are unbounded; the model stops at 64 bits. Saying so
        // here is what keeps a 65-bit value from being truncated into a
        // plausible-looking one.
        return if ob.extract::<i64>().is_ok() || ob.extract::<u64>().is_ok() {
            Kind::Int
        } else {
            Kind::IntegerTooWide
        };
    }
    if ob.is_instance_of::<PyString>() {
        return Kind::String;
    }
    if ob.is_instance_of::<PyFloat>() {
        return Kind::Float;
    }
    if ob.is_instance_of::<PyDict>() {
        return Kind::Object;
    }
    if ob.is_instance_of::<PyList>() || ob.is_instance_of::<PyTuple>() {
        return Kind::Array;
    }
    let py = ob.py();
    // A date reaches the engine as its wire form, so the same rules apply to a
    // `datetime.date` as to the string a JSON payload would have carried.
    if matches!(ob.is_instance(bindings.datetime.bind(py)), Ok(true))
        || matches!(ob.is_instance(bindings.date.bind(py)), Ok(true))
    {
        return Kind::String;
    }
    Kind::Foreign
}

impl<'a, 'py> Input for PyInput<'a, 'py> {
    type Child<'x>
        = PyInput<'a, 'py>
    where
        Self: 'x;

    fn kind(&self) -> Kind {
        self.kind
    }

    fn as_bool(&self) -> Option<bool> {
        if self.kind == Kind::Bool {
            self.ob.extract().ok()
        } else {
            None
        }
    }

    fn as_int(&self) -> Option<Int> {
        if self.kind != Kind::Int {
            return None;
        }
        if let Ok(n) = self.ob.extract::<i64>() {
            return Some(Int::Signed(n));
        }
        self.ob.extract::<u64>().ok().map(Int::Unsigned)
    }

    fn as_f64(&self) -> Option<f64> {
        match self.kind {
            Kind::Float => self.ob.extract().ok(),
            Kind::Int => self.ob.extract::<f64>().ok(),
            _ => None,
        }
    }

    fn as_str(&self) -> Option<Cow<'_, str>> {
        if self.kind != Kind::String {
            return None;
        }
        if let Ok(s) = self.ob.cast::<PyString>() {
            // Borrowed where the interpreter can lend its buffer. Forcing an
            // owned String here would allocate on every call, and the validator
            // asks more than once per value.
            return s.to_cow().ok();
        }
        // A date or datetime: hand the engine the wire form and let its rules
        // decide, rather than teaching this file what a valid date is.
        let iso = self.ob.call_method0("isoformat").ok()?;
        iso.extract::<String>().ok().map(Cow::Owned)
    }

    fn len(&self) -> usize {
        match self.kind {
            Kind::Array | Kind::Object => self.ob.len().unwrap_or(0),
            _ => 0,
        }
    }

    fn item(&self, index: usize) -> Option<Self::Child<'_>> {
        if self.kind != Kind::Array {
            return None;
        }
        self.ob.get_item(index).ok().map(|v| self.child(v))
    }

    fn slot(&self, key: &str) -> Slot<Self::Child<'_>> {
        if self.kind != Kind::Object {
            return Slot::Absent;
        }
        let Ok(dict) = self.ob.cast::<PyDict>() else {
            return Slot::Absent;
        };
        // The interned key when the schema declared it, which is every call
        // that matters; a fresh string only for a name the cache never saw.
        let found = match self.bindings.key(self.ob.py(), key) {
            Some(interned) => dict.get_item(interned),
            None => dict.get_item(key),
        };
        match found {
            Ok(Some(v)) if v.is_none() => Slot::Null,
            Ok(Some(v)) => Slot::Present(self.child(v)),
            _ => Slot::Absent,
        }
    }

    fn each_key(&self, f: &mut dyn FnMut(&str)) {
        if self.kind != Kind::Object {
            return;
        }
        let Ok(dict) = self.ob.cast::<PyDict>() else {
            return;
        };
        for (k, _) in dict.iter() {
            match k.extract::<String>() {
                Ok(s) => f(&s),
                // A non-string key matches no field, so reporting it as unknown
                // beats letting it through unseen.
                Err(_) => f(&k.str().map_or_else(|_| "<key>".into(), |s| s.to_string())),
            }
        }
    }
}

/// Builds the result from the payload and the schema.
///
/// Only values that need converting are rebuilt. A string, an int or a bool
/// comes back as the very object that arrived, so the common case costs a
/// refcount instead of an allocation and a copy.
struct Out<'a, 'py> {
    py: Python<'py>,
    schema: &'a seam_core::Schema,
    bindings: &'a Bindings,
}

impl<'py> Out<'_, 'py> {
    fn object(&self, ty: &ObjectType, ob: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let out = PyDict::new(self.py);
        let Ok(dict) = ob.cast::<PyDict>() else {
            return Ok(out.into_any());
        };

        for field in &ty.fields {
            // An absent key stays absent. `"bio" in result` is how a caller
            // reads absence, and writing None here would collapse it into null.
            match self.bindings.key(self.py, &field.name) {
                Some(key) => {
                    if let Ok(Some(v)) = dict.get_item(key) {
                        out.set_item(key, self.value(&field.ty, &v)?)?;
                    }
                }
                None => {
                    if let Ok(Some(v)) = dict.get_item(&field.name) {
                        out.set_item(&field.name, self.value(&field.ty, &v)?)?;
                    }
                }
            }
        }

        if !ty.deny_unknown_fields {
            for (k, v) in dict.iter() {
                if !matches!(k.extract::<String>(), Ok(name) if ty.field(&name).is_some()) {
                    out.set_item(k, v)?;
                }
            }
        }

        Ok(out.into_any())
    }

    fn value(&self, ty: &Type, ob: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        if ob.is_none() {
            return Ok(ob.clone());
        }
        match ty {
            Type::Date => self.date(ob),
            Type::DateTime => self.datetime(ob),
            Type::Array { item, .. } if needs_rebuilding(item, self.schema) => {
                let list = PyList::empty(self.py);
                for v in ob.try_iter()? {
                    list.append(self.value(item, &v?)?)?;
                }
                Ok(list.into_any())
            }
            Type::Object(obj) => self.object(obj, ob),
            Type::Ref(name) => match self.schema.get(name) {
                Some(obj) => self.object(obj, ob),
                None => Ok(ob.clone()),
            },
            // Nothing to convert: hand back what arrived.
            _ => Ok(ob.clone()),
        }
    }

    fn date(&self, ob: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let date_cls = self.bindings.date.bind(self.py);
        // Already a `date` and not a `datetime`: nothing to build.
        if matches!(ob.is_instance(date_cls), Ok(true))
            && !matches!(
                ob.is_instance(self.bindings.datetime.bind(self.py)),
                Ok(true)
            )
        {
            return Ok(ob.clone());
        }
        let s: String = ob.extract()?;
        date_cls.call_method1("fromisoformat", (s,))
    }

    fn datetime(&self, ob: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let datetime_cls = self.bindings.datetime.bind(self.py);
        if matches!(ob.is_instance(datetime_cls), Ok(true)) {
            return Ok(ob.clone());
        }
        let s: String = ob.extract()?;
        datetime_cls.call_method1("fromisoformat", (python_iso(&s),))
    }
}

impl<'py> Out<'_, 'py> {
    fn object_from_value(&self, ty: &ObjectType, v: &Value) -> PyResult<Bound<'py, PyAny>> {
        let out = PyDict::new(self.py);
        let Value::Object(map) = v else {
            return Ok(out.into_any());
        };
        for field in &ty.fields {
            // Absent stays absent, the same as on the dict path.
            if let Some(found) = map.get(&field.name) {
                let value = self.emit(&field.ty, found)?;
                match self.bindings.key(self.py, &field.name) {
                    Some(key) => out.set_item(key, value)?,
                    None => out.set_item(&field.name, value)?,
                }
            }
        }
        if !ty.deny_unknown_fields {
            for (k, extra) in map {
                if ty.field(k).is_none() {
                    out.set_item(k, self.plain(extra)?)?;
                }
            }
        }
        Ok(out.into_any())
    }

    fn emit(&self, ty: &Type, v: &Value) -> PyResult<Bound<'py, PyAny>> {
        if matches!(v, Value::Null) {
            return Ok(self.py.None().into_bound(self.py));
        }
        match (ty, v) {
            (Type::Date, Value::String(s)) => self
                .bindings
                .date
                .bind(self.py)
                .call_method1("fromisoformat", (s,)),
            (Type::DateTime, Value::String(s)) => self
                .bindings
                .datetime
                .bind(self.py)
                .call_method1("fromisoformat", (python_iso(s),)),
            (Type::Array { item, .. }, Value::Array(items)) => {
                let list = PyList::empty(self.py);
                for each in items {
                    list.append(self.emit(item, each)?)?;
                }
                Ok(list.into_any())
            }
            (Type::Object(obj), _) => self.object_from_value(obj, v),
            (Type::Ref(name), _) => match self.schema.get(name) {
                Some(obj) => self.object_from_value(obj, v),
                None => self.plain(v),
            },
            _ => self.plain(v),
        }
    }

    fn plain(&self, v: &Value) -> PyResult<Bound<'py, PyAny>> {
        Ok(match v {
            Value::Null => self.py.None().into_bound(self.py),
            Value::Bool(b) => b.into_pyobject(self.py)?.to_owned().into_any(),
            Value::Int(Int::Signed(n)) => n.into_pyobject(self.py)?.into_any(),
            Value::Int(Int::Unsigned(n)) => n.into_pyobject(self.py)?.into_any(),
            Value::Float(f) => f.into_pyobject(self.py)?.into_any(),
            Value::String(s) => s.into_pyobject(self.py)?.into_any(),
            Value::Array(items) => {
                let list = PyList::empty(self.py);
                for each in items {
                    list.append(self.plain(each)?)?;
                }
                list.into_any()
            }
            Value::Object(map) => {
                let d = PyDict::new(self.py);
                for (k, each) in map {
                    d.set_item(k, self.plain(each)?)?;
                }
                d.into_any()
            }
            // Validation rejects this before the result is built.
            Value::IntTooWide => self.py.None().into_bound(self.py),
        })
    }
}

/// Whether a type's values ever come back as something other than what arrived.
///
/// Answering no lets a whole array or field skip reconstruction entirely.
fn needs_rebuilding(ty: &Type, schema: &seam_core::Schema) -> bool {
    match ty {
        Type::Date | Type::DateTime | Type::Object(_) => true,
        Type::Array { item, .. } => needs_rebuilding(item, schema),
        Type::Ref(name) => schema.get(name).is_some(),
        _ => false,
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
    m.add_class::<Issues>()?;
    m.add("ParseError", py.get_type::<ParseError>())?;
    m.add("__version__", seam_core::VERSION)?;
    Ok(())
}
