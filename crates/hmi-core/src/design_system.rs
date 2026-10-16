//! Validation and scalar style resolution for the portable design system model.
//! Components are materialized into ordinary widgets by an atomic editor transaction.
use crate::{Project, ValidationIssue};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashSet};

const VISUAL: &[&str] = &[
    "color",
    "background",
    "backgroundColor",
    "borderColor",
    "fontFamily",
    "fontSize",
    "fontWeight",
    "lineHeight",
    "letterSpacing",
    "opacity",
    "borderRadius",
    "radius",
    "boxShadow",
    "shadow",
    "padding",
    "gap",
    "width",
    "height",
    "typography",
];
fn tokens(project: &Project) -> BTreeMap<String, Value> {
    let mut tokens = BTreeMap::new();
    let Some(system) = project.design_system.as_ref() else {
        return tokens;
    };
    let Some(sets) = system["tokenSets"].as_object() else {
        return tokens;
    };
    // Qualified references pin a set and remain usable across imported themes.
    for (id, set) in sets {
        if let Some(values) = set["tokens"].as_object() {
            for (name, value) in values {
                tokens.insert(format!("{id}.{name}"), value.clone());
            }
        }
    }
    let active = system["activeThemeId"]
        .as_str()
        .and_then(|id| system["themes"][id]["tokenSets"].as_array());
    let ids: Vec<_> = active
        .map(|v| v.iter().filter_map(Value::as_str).collect())
        .unwrap_or_else(|| sets.keys().map(String::as_str).collect());
    for id in ids {
        if let Some(values) = sets.get(id).and_then(|s| s["tokens"].as_object()) {
            for (name, value) in values {
                let qualified = format!("{id}.{name}");
                tokens.insert(qualified.clone(), value.clone());
                tokens.insert(
                    name.clone(),
                    json!({"$type":value["$type"],"$value":format!("{{{qualified}}}")}),
                );
            }
        }
    }
    tokens
}
fn resolve(
    source: &BTreeMap<String, Value>,
    key: &str,
    active: &mut HashSet<String>,
) -> Result<(String, Value), String> {
    if !active.insert(key.into()) {
        return Err(format!("Design token alias cycle: {key}"));
    }
    let token = source
        .get(key)
        .ok_or_else(|| format!("Unresolved design token: {key}"))?;
    let kind = token["$type"].as_str().ok_or("Token requires $type")?;
    let mut value = token
        .get("$value")
        .cloned()
        .ok_or("Token requires $value")?;
    if let Some(alias) = value
        .as_str()
        .and_then(|s| s.strip_prefix('{'))
        .and_then(|s| s.strip_suffix('}'))
    {
        let local = key.split_once('.').map(|(set, _)| format!("{set}.{alias}"));
        let reference = local
            .filter(|name| source.contains_key(name))
            .unwrap_or_else(|| alias.into());
        let (target, resolved) = resolve(source, &reference, active)?;
        if target != kind {
            return Err(format!("Design token alias type mismatch: {key}"));
        }
        value = resolved;
    }
    let valid = match kind {
        "color" => value.is_string() || value.is_object(),
        "dimension" | "radius" => {
            value.is_number()
                || value.is_string()
                || (value["value"].is_number() && value["unit"].is_string())
        }
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "typography" => value.is_object(),
        "shadow" => value.is_string() || value.is_object() || value.is_array(),
        "fontFamily" => value.is_string() || value.is_array(),
        "opacity" => value.as_f64().is_some_and(|v| (0.0..=1.0).contains(&v)),
        _ => false,
    };
    if !valid {
        return Err(format!("Invalid {kind} design token: {key}"));
    }
    active.remove(key);
    Ok((kind.into(), value))
}
pub fn resolve_style_token(project: &Project, reference: &str) -> Option<Value> {
    let (kind, value) = resolve(&tokens(project), reference, &mut HashSet::new()).ok()?;
    normalize(&value, &kind)
}
fn normalize(value: &Value, kind: &str) -> Option<Value> {
    if ["dimension", "radius"].contains(&kind) {
        if let Some(text) = value.as_str() {
            for (suffix, factor) in [("px", 1.0), ("rem", 16.0)] {
                if let Some(number) = text
                    .strip_suffix(suffix)
                    .and_then(|n| n.parse::<f64>().ok())
                    .filter(|n| n.is_finite())
                {
                    return Some(json!(number * factor));
                }
            }
        }
    }
    if value["unit"] == "px" {
        return value.get("value").cloned();
    }
    if value["unit"] == "rem" {
        return value["value"].as_f64().map(|v| json!(v * 16.0));
    }
    if value["colorSpace"] == "srgb" {
        if let Some(components) = value["components"].as_array().filter(|v| v.len() == 3) {
            let rgb: Vec<_> = components
                .iter()
                .filter_map(Value::as_f64)
                .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as i64)
                .collect();
            if rgb.len() == 3 {
                let alpha = value["alpha"].as_f64().unwrap_or(1.0);
                return Some(json!(if alpha == 1.0 {
                    format!("rgb({} {} {})", rgb[0], rgb[1], rgb[2])
                } else {
                    format!("rgb({} {} {} / {})", rgb[0], rgb[1], rgb[2], alpha)
                }));
            }
        }
    }
    if kind == "shadow" && (value.is_array() || value.get("offsetX").is_some()) {
        let single = vec![value.clone()];
        let shadows = value.as_array().unwrap_or(&single);
        let mut parts = Vec::new();
        for shadow in shadows {
            let mut dims = Vec::new();
            for field in ["offsetX", "offsetY", "blur", "spread"] {
                if field == "spread" && shadow.get(field).is_none() {
                    continue;
                }
                let v = normalize(&shadow[field], "dimension")?;
                dims.push(format!("{}px", v.as_f64()?));
            }
            let color = normalize(&shadow["color"], "color")?;
            parts.push(format!("{} {}", dims.join(" "), color.as_str()?));
        }
        return Some(json!(parts.join(", ")));
    }
    if value.get("unit").is_some() || value.get("colorSpace").is_some() {
        return None;
    }
    Some(value.clone())
}
fn component_graph_valid(
    id: &str,
    components: &Map<String, Value>,
    active: &mut HashSet<String>,
    complete: &mut HashSet<String>,
) -> bool {
    if complete.contains(id) {
        return true;
    }
    if active.len() >= 64 || !active.insert(id.into()) {
        return false;
    }
    let Some(widgets) = components.get(id).and_then(|c| c["widgets"].as_array()) else {
        return false;
    };
    let parents: BTreeMap<_, _> = widgets
        .iter()
        .filter_map(|w| Some((w["id"].as_str()?, w["parentId"].as_str())))
        .collect();
    for widget in widgets {
        let mut seen = HashSet::new();
        let mut next = widget["id"].as_str();
        while let Some(key) = next {
            if !seen.insert(key) {
                return false;
            }
            next = parents.get(key).copied().flatten();
        }
        if let Some(child) = widget["componentInstance"]["componentId"].as_str() {
            if !component_graph_valid(child, components, active, complete) {
                return false;
            }
        }
    }
    active.remove(id);
    complete.insert(id.into());
    true
}
/// Match the Vue visual projection, including a typography token's individual fields.
pub fn resolve_widget_style(
    project: &Project,
    widget: &crate::Widget,
    property: &str,
) -> Option<Value> {
    let source = tokens(project);
    let mut result = None;
    let mut entries: Vec<_> = widget.style_tokens.iter().collect();
    entries.sort_by_key(|(_, key)| {
        resolve(&source, key, &mut HashSet::new())
            .map(|(kind, _)| kind != "typography")
            .unwrap_or(true)
    });
    for (target, key) in entries {
        if widget.bindings.contains_key(target) {
            continue;
        }
        let (kind, value) = resolve(&source, key, &mut HashSet::new()).ok()?;
        if kind == "typography"
            && [
                "fontFamily",
                "fontSize",
                "fontWeight",
                "lineHeight",
                "letterSpacing",
            ]
            .contains(&property)
        {
            if !widget.bindings.contains_key(property) {
                if let Some(field) = value.get(property) {
                    result = normalize(
                        field,
                        if ["fontSize", "letterSpacing"].contains(&property) {
                            "dimension"
                        } else {
                            ""
                        },
                    );
                }
            }
        } else if target == property {
            result = normalize(&value, &kind);
        }
    }
    result
}
pub fn validate(project: &Project) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    let mut issue = |path: String, message: String| {
        issues.push(ValidationIssue {
            severity: "error".into(),
            path,
            message,
        })
    };
    for page in &project.pages {
        let layouts = page
            .layout
            .iter()
            .chain(page.widgets.iter().filter_map(|w| w.layout.as_ref()));
        for layout in layouts {
            let valid = layout
                .gap_x
                .into_iter()
                .chain(layout.gap_y)
                .chain(
                    layout
                        .padding_sides
                        .iter()
                        .flat_map(|m| m.values().copied()),
                )
                .all(|v| v.is_finite() && v >= 0.0)
                && layout.direction.as_deref().map_or(true, |s| {
                    ["row", "column", "row-reverse", "column-reverse"].contains(&s)
                })
                && layout.justify.as_deref().map_or(true, |s| {
                    [
                        "start",
                        "center",
                        "end",
                        "space-between",
                        "space-around",
                        "space-evenly",
                    ]
                    .contains(&s)
                })
                && layout
                    .align
                    .as_deref()
                    .map_or(true, |s| ["start", "center", "end", "stretch"].contains(&s))
                && layout
                    .column_tracks
                    .iter()
                    .chain(layout.row_tracks.iter())
                    .flatten()
                    .all(|t| match t.kind.as_str() {
                        "auto" => true,
                        "px" => t.value.is_some_and(|v| v.is_finite() && v >= 0.0),
                        "fr" => t.value.is_some_and(|v| v.is_finite() && v > 0.0),
                        _ => false,
                    });
            if !valid {
                issue(
                    format!("pages.{}.layout", page.id),
                    "Invalid flexible layout".into(),
                );
            }
        }
        for widget in &page.widgets {
            if let Some(size) = widget.layout_item.as_ref().and_then(|i| i.size.as_ref()) {
                if !size.iter().all(|(axis, size)| {
                    ["width", "height"].contains(&axis.as_str())
                        && ["fixed", "fit", "fill"].contains(&size.mode.as_str())
                        && size.value.map_or(true, |v| v.is_finite() && v >= 0.0)
                }) {
                    issue(
                        format!("widgets.{}.layoutItem.size", widget.id),
                        "Invalid sizing mode".into(),
                    );
                }
            }
        }
    }
    let Some(system) = project.design_system.as_ref() else {
        for page in &project.pages {
            for widget in &page.widgets {
                if !widget.style_tokens.is_empty() || widget.component_instance.is_some() {
                    issue(
                        format!("widgets.{}", widget.id),
                        "Widget requires designSystem".into(),
                    );
                }
            }
        }
        return issues;
    };
    if project.schema_version < 4 {
        issue(
            "schemaVersion".into(),
            "Design systems require schema 4".into(),
        );
    }
    if system["version"] != 1 {
        issue(
            "designSystem.version".into(),
            "Unsupported design system version".into(),
        );
    }
    for key in ["tokenSets", "themes", "components", "libraries"] {
        if !system[key].is_object() {
            issue(format!("designSystem.{key}"), "Expected an object".into());
        }
    }
    let empty = Map::new();
    let sets = system["tokenSets"].as_object().unwrap_or(&empty);
    if let Some(id) = system["activeThemeId"].as_str() {
        if system["themes"].get(id).is_none() {
            issue("designSystem.activeThemeId".into(), "Unknown theme".into());
        }
    }
    for (id, theme) in system["themes"].as_object().unwrap_or(&empty) {
        if !theme["tokenSets"].as_array().is_some_and(|ids| {
            ids.iter()
                .all(|id| id.as_str().is_some_and(|id| sets.contains_key(id)))
        }) {
            issue(
                format!("designSystem.themes.{id}"),
                "Theme references an unknown token set".into(),
            );
        }
    }
    // Validate all sets, including currently inactive themes.
    let mut all = BTreeMap::new();
    for (id, set) in sets {
        for (key, token) in set["tokens"].as_object().unwrap_or(&empty) {
            all.insert(format!("{id}.{key}"), token.clone());
        }
    }
    for key in all.keys() {
        if let Err(error) = resolve(&all, key, &mut HashSet::new()) {
            issue(format!("designSystem.tokenSets.{key}"), error);
        }
    }
    let components = system["components"].as_object().unwrap_or(&empty);
    for (id, component) in components {
        if !component_graph_valid(id, components, &mut HashSet::new(), &mut HashSet::new()) {
            issue(
                format!("designSystem.components.{id}"),
                "Component tree contains a cycle or missing nested component".into(),
            );
        }
        if !component["widgets"].as_array().is_some_and(|widgets| {
            let ids: HashSet<_> = widgets.iter().filter_map(|w| w["id"].as_str()).collect();
            ids.len() == widgets.len()
                && widgets.iter().any(|w| w["id"] == component["rootWidgetId"])
                && widgets.iter().all(|w| {
                    w["parentId"]
                        .as_str()
                        .map_or(true, |parent| ids.contains(parent))
                })
        }) {
            issue(
                format!("designSystem.components.{id}"),
                "Component requires a root widget".into(),
            );
        }
    }
    for page in &project.pages {
        for widget in &page.widgets {
            for (property, key) in &widget.style_tokens {
                if !VISUAL.contains(&property.as_str())
                    || resolve_style_token(project, key).is_none()
                {
                    issue(
                        format!("widgets.{}.styleTokens.{property}", widget.id),
                        "Invalid visual property or missing token".into(),
                    );
                }
            }
            if let Some(instance) = &widget.component_instance {
                let component = instance["componentId"]
                    .as_str()
                    .and_then(|id| components.get(id));
                if component.is_none()
                    || instance["rootWidgetId"] != widget.id
                    || !instance["widgetIdMap"].as_object().is_some_and(|map| {
                        let unique: HashSet<_> = map.values().filter_map(Value::as_str).collect();
                        unique.len() == map.len()
                            && map
                                .values()
                                .all(|id| page.widgets.iter().any(|w| id == &w.id))
                            && component.is_some_and(|c| {
                                map.get(c["rootWidgetId"].as_str().unwrap_or(""))
                                    == Some(&json!(widget.id))
                                    && c["widgets"].as_array().is_some_and(|items| {
                                        items.iter().all(|w| {
                                            w["id"].as_str().is_some_and(|id| map.contains_key(id))
                                        })
                                    })
                            })
                    })
                {
                    issue(
                        format!("widgets.{}.componentInstance", widget.id),
                        "Invalid component or widget identity mapping".into(),
                    );
                }
            }
        }
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    fn project() -> Project {
        serde_json::from_value(json!({
        "schemaVersion":4,"id":"design-tests","name":"Design","width":800,"height":600,
        "pages":[{"id":"main","name":"Main","widgets":[{"id":"label","kind":"label","name":"Label","x":0,"y":0,"width":100,"height":30,"zIndex":0,"props":{},"styleTokens":{"color":"color.primary","fontSize":"space","boxShadow":"shadow"}}]}],
        "designSystem":{"version":1,"tokenSets":{
            "base":{"id":"base","name":"Base","tokens":{
                "color.primary":{"$type":"color","$value":"#123456"},
                "color.alias":{"$type":"color","$value":"{color.primary}"},
                "space":{"$type":"dimension","$value":"1.5rem"},
                "shadow":{"$type":"shadow","$value":{"offsetX":0,"offsetY":{"value":2,"unit":"px"},"blur":4,"color":{"colorSpace":"srgb","components":[0.5,0,1],"alpha":0.5}}}
            }},
            "dark":{"id":"dark","name":"Dark","tokens":{"color.primary":{"$type":"color","$value":"#ffffff"}}}
        },"themes":{"dark":{"id":"dark","name":"Dark","tokenSets":["base","dark"]}},"activeThemeId":"dark","components":{},"libraries":{}}
    })).unwrap()
    }
    #[test]
    fn aliases_keep_same_set_identity_while_themes_override_unqualified_names() {
        let p = project();
        assert!(validate(&p).is_empty(), "{:?}", validate(&p));
        assert_eq!(
            resolve_style_token(&p, "color.primary"),
            Some(json!("#ffffff"))
        );
        assert_eq!(
            resolve_style_token(&p, "base.color.alias"),
            Some(json!("#123456"))
        );
        let mut light = p.clone();
        light.design_system.as_mut().unwrap()["themes"]["dark"]["tokenSets"] = json!(["base"]);
        assert_eq!(
            resolve_style_token(&light, "dark.color.primary"),
            Some(json!("#ffffff"))
        );
        assert_eq!(resolve_style_token(&p, "space"), Some(json!(24.0)));
        assert_eq!(
            resolve_style_token(&p, "shadow"),
            Some(json!("0px 2px 4px rgb(128 0 255 / 0.5)"))
        );
    }
    #[test]
    fn broken_alias_type_and_cycles_are_rejected() {
        let mut p = project();
        p.design_system.as_mut().unwrap()["tokenSets"]["base"]["tokens"]["space"]["$value"] =
            json!("{color.primary}");
        assert!(validate(&p)
            .iter()
            .any(|i| i.message.contains("type mismatch")));
        p.design_system.as_mut().unwrap()["tokenSets"]["base"]["tokens"]["color.primary"]
            ["$value"] = json!("{color.alias}");
        assert!(validate(&p).iter().any(|i| i.message.contains("cycle")));
    }
    #[test]
    fn typography_projection_and_layout_metadata_roundtrip() {
        let mut p = project();
        p.design_system.as_mut().unwrap()["tokenSets"]["base"]["tokens"]["heading"] = json!({"$type":"typography","$value":{"fontSize":{"value":2,"unit":"rem"},"fontFamily":"Segoe UI","fontWeight":600}});
        p.pages[0].widgets[0]
            .style_tokens
            .insert("fontFamily".into(), "heading".into());
        p.pages[0].layout=Some(serde_json::from_value(json!({"type":"grid","gapX":8,"paddingSides":{"left":12},"columnTracks":[{"kind":"fr","value":1},{"kind":"px","value":240}]})).unwrap());
        assert_eq!(
            resolve_widget_style(&p, &p.pages[0].widgets[0], "fontSize"),
            Some(json!(24.0))
        );
        p.pages[0].widgets[0].style_tokens.remove("fontSize");
        assert_eq!(
            resolve_widget_style(&p, &p.pages[0].widgets[0], "fontSize"),
            Some(json!(32.0))
        );
        let restored: Project = serde_json::from_value(serde_json::to_value(&p).unwrap()).unwrap();
        assert!(validate(&restored).is_empty());
        assert_eq!(restored.design_system, p.design_system);
        p.pages[0]
            .layout
            .as_mut()
            .unwrap()
            .column_tracks
            .as_mut()
            .unwrap()[0]
            .value = Some(-1.0);
        assert!(validate(&p).iter().any(|i| i.message.contains("layout")));
    }
    #[test]
    fn component_instances_require_complete_injective_mapping() {
        let mut p = project();
        p.design_system.as_mut().unwrap()["components"]["card"] = json!({"id":"card","name":"Card","rootWidgetId":"source","widgets":[{"id":"source"},{"id":"child","parentId":"source"}]});
        p.pages[0].widgets[0].component_instance = Some(
            json!({"componentId":"card","instanceId":"instance","rootWidgetId":"label","widgetIdMap":{"source":"label","child":"label"}}),
        );
        assert!(validate(&p)
            .iter()
            .any(|i| i.path.ends_with("componentInstance")));
    }
    #[test]
    fn component_parent_and_nested_definition_cycles_are_rejected() {
        let mut p = project();
        p.design_system.as_mut().unwrap()["components"]["card"] = json!({"id":"card","rootWidgetId":"root","widgets":[{"id":"root","parentId":"child"},{"id":"child","parentId":"root"}]});
        assert!(validate(&p).iter().any(|i| i.message.contains("cycle")));
        p.design_system.as_mut().unwrap()["components"]["card"] = json!({"id":"card","rootWidgetId":"root","widgets":[{"id":"root","componentInstance":{"componentId":"card"}}]});
        assert!(validate(&p).iter().any(|i| i.message.contains("cycle")));
    }
}
