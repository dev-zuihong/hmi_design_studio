use crate::{CoreError, CoreResult};
use serde_json::Value;

/// The supported JSON Schema subset is validated before use. Unsupported
/// assertions are rejected instead of silently accepting unsafe command values.
pub fn validate_schema(schema: &Value, value: &Value) -> CoreResult<()> {
    validate_schema_definition(schema)?;
    validate(schema, value, "$", 0)
}
pub(crate) fn validate_schema_definition(schema: &Value) -> CoreResult<()> {
    definition(schema, "$", 0)
}
fn invalid(path: &str, message: impl std::fmt::Display) -> CoreError {
    CoreError::new("SCHEMA_INVALID", format!("{path}: {message}"))
}
fn mismatch(path: &str, message: impl std::fmt::Display) -> CoreError {
    CoreError::new("SCHEMA_VALUE", format!("{path}: {message}"))
}
fn valid_type(name: &str) -> bool {
    [
        "object", "array", "string", "number", "integer", "boolean", "null",
    ]
    .contains(&name)
}
fn definition(schema: &Value, path: &str, depth: usize) -> CoreResult<()> {
    if depth > 32 {
        return Err(invalid(path, "Schema nesting exceeds 32"));
    }
    if schema.is_null() || schema.is_boolean() {
        return Ok(());
    }
    let object = schema
        .as_object()
        .ok_or_else(|| invalid(path, "Schema must be an object or boolean"))?;
    for (key, value) in object {
        let here = format!("{path}.{key}");
        match key.as_str() {
            "type" => {
                if !value.as_str().is_some_and(valid_type)
                    && !value.as_array().is_some_and(|types| {
                        !types.is_empty()
                            && types.iter().all(|t| t.as_str().is_some_and(valid_type))
                    })
                {
                    return Err(invalid(&here, "Unsupported type"));
                }
            }
            "properties" => {
                let properties = value
                    .as_object()
                    .ok_or_else(|| invalid(&here, "properties must be an object"))?;
                for (name, property) in properties {
                    definition(property, &format!("{here}.{name}"), depth + 1)?;
                }
            }
            "required" => {
                if !value
                    .as_array()
                    .is_some_and(|names| names.iter().all(Value::is_string))
                {
                    return Err(invalid(
                        &here,
                        "required must be an array of property names",
                    ));
                }
            }
            "additionalProperties" | "items" => definition(value, &here, depth + 1)?,
            "enum" => {
                if !value.as_array().is_some_and(|items| !items.is_empty()) {
                    return Err(invalid(&here, "enum must be a nonempty array"));
                }
            }
            "minimum" | "maximum" | "exclusiveMinimum" | "exclusiveMaximum" => {
                if !value.is_number() {
                    return Err(invalid(&here, "Bound must be a number"));
                }
            }
            "minLength" | "maxLength" | "minItems" | "maxItems" => {
                if value.as_u64().is_none() {
                    return Err(invalid(&here, "Length must be a nonnegative integer"));
                }
            }
            "const" | "title" | "description" | "default" | "examples" | "$schema" | "$comment"
            | "readOnly" | "writeOnly" | "deprecated" => (),
            _ => {
                return Err(invalid(
                    &here,
                    format!("Unsupported JSON Schema keyword: {key}"),
                ))
            }
        }
    }
    Ok(())
}
fn matches_type(value: &Value, name: &str) -> bool {
    match name {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.as_f64().is_some_and(|n| n.fract() == 0.0),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => false,
    }
}
fn equal(left: &Value, right: &Value) -> bool {
    if left.is_number() && right.is_number() {
        // Integer comparisons retain precision; floats use JSON's numeric value.
        if let (Some(a), Some(b)) = (left.as_i64(), right.as_i64()) {
            return a == b;
        }
        if let (Some(a), Some(b)) = (left.as_u64(), right.as_u64()) {
            return a == b;
        }
        return left.as_f64() == right.as_f64();
    }
    left == right
}
fn validate(schema: &Value, value: &Value, path: &str, depth: usize) -> CoreResult<()> {
    if depth > 32 {
        return Err(mismatch(path, "Value nesting exceeds schema limit"));
    }
    if schema == &Value::Bool(false) {
        return Err(mismatch(path, "Value is forbidden by schema"));
    }
    let Some(object) = schema.as_object() else {
        return Ok(());
    };
    if let Some(types) = object.get("type") {
        let matches = types.as_str().is_some_and(|t| matches_type(value, t))
            || types
                .as_array()
                .is_some_and(|ts| ts.iter().any(|t| matches_type(value, t.as_str().unwrap())));
        if !matches {
            return Err(mismatch(path, format!("Value does not match type {types}")));
        }
    }
    if object
        .get("enum")
        .and_then(Value::as_array)
        .is_some_and(|items| !items.iter().any(|item| equal(value, item)))
    {
        return Err(mismatch(path, "Value is outside enum"));
    }
    if object
        .get("const")
        .is_some_and(|expected| !equal(value, expected))
    {
        return Err(mismatch(path, "Value differs from const"));
    }
    if let Some(number) = value.as_f64() {
        for (key, exclusive, lower) in [
            ("minimum", false, true),
            ("maximum", false, false),
            ("exclusiveMinimum", true, true),
            ("exclusiveMaximum", true, false),
        ] {
            if let Some(bound) = object.get(key).and_then(Value::as_f64) {
                let invalid = if lower {
                    number < bound || exclusive && number == bound
                } else {
                    number > bound || exclusive && number == bound
                };
                if invalid {
                    return Err(mismatch(path, format!("Value violates {key}={bound}")));
                }
            }
        }
    }
    if let Some(string) = value.as_str() {
        check_length(
            object,
            string.chars().count() as u64,
            "minLength",
            "maxLength",
            path,
        )?;
    }
    if let Some(array) = value.as_array() {
        check_length(object, array.len() as u64, "minItems", "maxItems", path)?;
        if let Some(items) = object.get("items") {
            for (index, item) in array.iter().enumerate() {
                validate(items, item, &format!("{path}[{index}]"), depth + 1)?;
            }
        }
    }
    if let Some(values) = value.as_object() {
        if let Some(required) = object.get("required").and_then(Value::as_array) {
            for name in required {
                let name = name.as_str().unwrap();
                if !values.contains_key(name) {
                    return Err(mismatch(path, format!("Missing required property {name}")));
                }
            }
        }
        let properties = object.get("properties").and_then(Value::as_object);
        for (name, value) in values {
            let property = properties
                .and_then(|p| p.get(name))
                .or_else(|| object.get("additionalProperties"));
            if let Some(property) = property {
                validate(property, value, &format!("{path}.{name}"), depth + 1)?;
            }
        }
    }
    Ok(())
}
fn check_length(
    schema: &serde_json::Map<String, Value>,
    length: u64,
    min: &str,
    max: &str,
    path: &str,
) -> CoreResult<()> {
    if schema
        .get(min)
        .and_then(Value::as_u64)
        .is_some_and(|n| length < n)
        || schema
            .get(max)
            .and_then(Value::as_u64)
            .is_some_and(|n| length > n)
    {
        return Err(mismatch(path, format!("Length violates {min}/{max}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn rejects_invalid_commands_before_transport() {
        let schema = json!({"type":"object","required":["target"],"properties":{"target":{"type":"number","minimum":-40,"maximum":200}},"additionalProperties":false});
        assert!(validate_schema(&schema, &json!({"target":35.5})).is_ok());
        for invalid in [
            json!({}),
            json!({"target":"35"}),
            json!({"target":201}),
            json!({"target":35,"extra":true}),
        ] {
            assert_eq!(
                validate_schema(&schema, &invalid).unwrap_err().code,
                "SCHEMA_VALUE"
            );
        }
    }
    #[test]
    fn nested_schemas_unicode_enum_and_unsupported_assertions() {
        let schema = json!({"type":"array","minItems":1,"maxItems":2,"items":{"type":"object","properties":{"mode":{"enum":["手动","自动"],"type":"string","maxLength":2}},"required":["mode"],"additionalProperties":false}});
        assert!(validate_schema(&schema, &json!([{"mode":"手动"}])).is_ok());
        assert!(validate_schema(&schema, &json!([{"mode":"未知"}])).is_err());
        assert!(validate_schema(&schema, &json!([])).is_err());
        assert_eq!(
            validate_schema(&json!({"pattern":"^[A-Z]+$"}), &json!("bad"))
                .unwrap_err()
                .code,
            "SCHEMA_INVALID"
        );
        assert!(validate_schema(&json!({"type":"integer"}), &json!(2.0)).is_ok());
        assert!(validate_schema(&json!({"type":"integer"}), &json!(2.5)).is_err());
    }
}
