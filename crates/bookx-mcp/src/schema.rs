//! A small JSON Schema checker for tool arguments: the subset the tool schemas use
//! (`type`, `required`, `properties`, `enum`, `minimum`, `maximum`, `minLength`, `maxLength`,
//! `items`, `maxItems`, `additionalProperties: false`).

use serde_json::Value;

/// Checks `value` against `schema`. The error names the offending argument.
pub fn validate(schema: &Value, value: &Value) -> Result<(), String> {
    check(schema, value, "arguments")
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn type_ok(want: &str, v: &Value) -> bool {
    match want {
        "number" => v.is_number(),
        "integer" => {
            v.is_i64()
                || v.is_u64()
                || v.as_f64()
                    .is_some_and(|f| f.fract() == 0.0 && f.abs() < 9e15)
        }
        other => type_name(v) == other,
    }
}

fn check(schema: &Value, v: &Value, at: &str) -> Result<(), String> {
    if let Some(t) = schema.get("type").and_then(Value::as_str) {
        if !type_ok(t, v) {
            return Err(format!("{at} must be {t}, got {}", type_name(v)));
        }
    }
    if let Some(options) = schema.get("enum").and_then(Value::as_array) {
        if !options.contains(v) {
            let list: Vec<String> = options
                .iter()
                .map(|o| {
                    o.as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| o.to_string())
                })
                .collect();
            return Err(format!("{at} must be one of: {}", list.join(", ")));
        }
    }
    if let Some(n) = v.as_f64() {
        if let Some(min) = schema.get("minimum").and_then(Value::as_f64) {
            if n < min {
                return Err(format!("{at} must be at least {min}"));
            }
        }
        if let Some(max) = schema.get("maximum").and_then(Value::as_f64) {
            if n > max {
                return Err(format!("{at} must be at most {max}"));
            }
        }
    }
    if let Some(s) = v.as_str() {
        let len = s.chars().count();
        if let Some(min) = schema.get("minLength").and_then(Value::as_u64) {
            if (len as u64) < min {
                return Err(format!("{at} must have at least {min} characters"));
            }
        }
        if let Some(max) = schema.get("maxLength").and_then(Value::as_u64) {
            if len as u64 > max {
                return Err(format!("{at} must have at most {max} characters"));
            }
        }
    }
    if let Some(items) = v.as_array() {
        if let Some(max) = schema.get("maxItems").and_then(Value::as_u64) {
            if items.len() as u64 > max {
                return Err(format!("{at} must have at most {max} items"));
            }
        }
        if let Some(item_schema) = schema.get("items") {
            for (i, it) in items.iter().enumerate() {
                check(item_schema, it, &format!("{at}[{i}]"))?;
            }
        }
    }
    if let Some(obj) = v.as_object() {
        if let Some(req) = schema.get("required").and_then(Value::as_array) {
            for r in req.iter().filter_map(Value::as_str) {
                if !obj.contains_key(r) {
                    return Err(format!("missing required argument `{r}`"));
                }
            }
        }
        let props = schema.get("properties").and_then(Value::as_object);
        if let Some(props) = props {
            for (k, val) in obj {
                match props.get(k) {
                    Some(s) => check(s, val, k)?,
                    None if schema.get("additionalProperties") == Some(&Value::Bool(false)) => {
                        return Err(format!("unknown argument `{k}`"));
                    }
                    None => {}
                }
            }
        }
    }
    Ok(())
}

/// Whether `schema` itself is well formed for the subset above (used by the tests over every
/// published tool).
pub fn schema_is_valid(schema: &Value) -> Result<(), String> {
    fn walk(s: &Value, at: &str) -> Result<(), String> {
        let Some(o) = s.as_object() else {
            return Err(format!("{at}: schema must be an object"));
        };
        if let Some(t) = o.get("type") {
            let ok = t.as_str().is_some_and(|t| {
                [
                    "object", "array", "string", "number", "integer", "boolean", "null",
                ]
                .contains(&t)
            });
            if !ok {
                return Err(format!("{at}: bad type {t}"));
            }
        }
        if let Some(r) = o.get("required") {
            let names = r.as_array().ok_or(format!("{at}: required is a list"))?;
            let props = o.get("properties").and_then(Value::as_object);
            for n in names {
                let n = n.as_str().ok_or(format!("{at}: required holds names"))?;
                if !props.is_some_and(|p| p.contains_key(n)) {
                    return Err(format!("{at}: required `{n}` is not a property"));
                }
            }
        }
        if let Some(p) = o.get("properties") {
            for (k, v) in p.as_object().ok_or(format!("{at}: properties is a map"))? {
                walk(v, &format!("{at}.{k}"))?;
            }
        }
        if let Some(i) = o.get("items") {
            walk(i, &format!("{at}[]"))?;
        }
        Ok(())
    }
    walk(schema, "schema")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn s() -> Value {
        json!({"type":"object","properties":{
            "id":{"type":"integer","minimum":1},
            "q":{"type":"string","maxLength":5},
            "mode":{"type":"string","enum":["a","b"]},
            "ids":{"type":"array","items":{"type":"integer"},"maxItems":2}},
            "required":["id"],"additionalProperties":false})
    }

    #[test]
    fn accepts_good_and_names_the_problem() {
        assert!(validate(&s(), &json!({"id":3,"q":"abc","mode":"a","ids":[1,2]})).is_ok());
        assert!(validate(&s(), &json!({})).unwrap_err().contains("`id`"));
        assert!(
            validate(&s(), &json!({"id":"x"}))
                .unwrap_err()
                .contains("integer")
        );
        assert!(
            validate(&s(), &json!({"id":0}))
                .unwrap_err()
                .contains("at least")
        );
        assert!(validate(&s(), &json!({"id":1,"q":"toolong"})).is_err());
        assert!(
            validate(&s(), &json!({"id":1,"mode":"z"}))
                .unwrap_err()
                .contains("a, b")
        );
        assert!(validate(&s(), &json!({"id":1,"ids":[1,2,3]})).is_err());
        assert!(validate(&s(), &json!({"id":1,"ids":["x"]})).is_err());
        assert!(
            validate(&s(), &json!({"id":1,"extra":1}))
                .unwrap_err()
                .contains("extra")
        );
        assert!(validate(&s(), &json!([])).is_err());
    }

    #[test]
    fn schema_checker_finds_mistakes() {
        assert!(schema_is_valid(&s()).is_ok());
        assert!(schema_is_valid(&json!({"type":"nope"})).is_err());
        assert!(schema_is_valid(&json!({"type":"object","required":["x"]})).is_err());
    }
}
