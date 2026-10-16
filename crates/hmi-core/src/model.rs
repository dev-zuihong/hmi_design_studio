use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Project {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub width: f64,
    pub height: f64,
    pub pages: Vec<Page>,
    #[serde(default)]
    pub connections: Vec<ConnectionConfig>,
    #[serde(default)]
    pub variables: Vec<Variable>,
    #[serde(default)]
    pub flows: Vec<Flow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub python: Option<PythonConfig>,
    #[serde(default)]
    pub assets: Vec<Asset>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub i18n: Option<I18nConfig>,
    #[serde(default)]
    pub actions: Vec<SharedAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PythonConfig {
    pub runtime_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_sha256: Option<String>,
    #[serde(default)]
    pub dependency_lock: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Asset {
    pub id: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Page {
    pub id: String,
    pub name: String,
    pub widgets: Vec<Widget>,
    #[serde(default)]
    pub layout: Option<LayoutSpec>,
    #[serde(default)]
    pub responsive: Option<Value>,
    #[serde(default)]
    pub guides: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Widget {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub z_index: i32,
    #[serde(default)]
    pub props: BTreeMap<String, Value>,
    #[serde(default)]
    pub bindings: BTreeMap<String, ValueRef>,
    #[serde(default)]
    pub events: BTreeMap<String, UiAction>,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub slot_id: Option<String>,
    #[serde(default)]
    pub layout: Option<LayoutSpec>,
    #[serde(default)]
    pub layout_item: Option<LayoutItem>,
    #[serde(default)]
    pub slots: Vec<WidgetSlot>,
    #[serde(default)]
    pub custom_properties: BTreeMap<String, CustomProperty>,
    #[serde(default)]
    pub editor: Option<EditorState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LayoutSpec {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub gap: Option<f64>,
    #[serde(default)]
    pub padding: Option<f64>,
    #[serde(default)]
    pub columns: Option<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LayoutItem {
    #[serde(default)]
    pub grow: Option<f64>,
    #[serde(default)]
    pub min_width: Option<f64>,
    #[serde(default)]
    pub max_width: Option<f64>,
    #[serde(default)]
    pub min_height: Option<f64>,
    #[serde(default)]
    pub max_height: Option<f64>,
    #[serde(default)]
    pub align: Option<String>,
    #[serde(default)]
    pub row: Option<u32>,
    #[serde(default)]
    pub column: Option<u32>,
    #[serde(default)]
    pub row_span: Option<u32>,
    #[serde(default)]
    pub column_span: Option<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WidgetSlot {
    pub id: String,
    pub title: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditorState {
    #[serde(default)]
    pub locked: Option<bool>,
    #[serde(default)]
    pub hidden: Option<bool>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CustomProperty {
    #[serde(rename = "type")]
    pub value_type: String,
    pub value: Value,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub binding: Option<ValueRef>,
    #[serde(default)]
    pub event: Option<UiAction>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct I18nConfig {
    pub default_locale: String,
    #[serde(default)]
    pub locales: Vec<String>,
    #[serde(default)]
    pub messages: BTreeMap<String, BTreeMap<String, String>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SharedAction {
    pub id: String,
    pub text: Value,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub shortcut: Option<String>,
    #[serde(default)]
    pub enabled: Option<Value>,
    #[serde(default)]
    pub visible: Option<Value>,
    pub action: UiAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "camelCase", deny_unknown_fields)]
pub enum ValueRef {
    Literal { value: Value },
    Message { path: String },
    Variable { key: String },
    Input { key: String },
    Point { key: String },
    Property { key: String, property: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiAction {
    pub kind: String,
    pub flow_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_node_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Variable {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub value_type: String,
    pub initial: Value,
    #[serde(default)]
    pub options: Vec<String>,
}

impl Variable {
    pub fn accepts(&self, value: &Value) -> bool {
        match self.value_type.as_str() {
            "number" => value.is_number(),
            "string" => value.is_string(),
            "boolean" => value.is_boolean(),
            "enum" => value
                .as_str()
                .is_some_and(|v| self.options.iter().any(|o| o == v)),
            "array" => value.is_array(),
            "object" => value.is_object(),
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectionConfig {
    pub id: String,
    pub name: String,
    pub port: String,
    pub baud_rate: u32,
    pub data_bits: u8,
    pub stop_bits: u8,
    pub parity: String,
    pub protocol_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<String>,
    #[serde(default)]
    pub protocol_config: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Flow {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub timeout_ms: u64,
    pub nodes: Vec<FlowNode>,
    pub edges: Vec<FlowEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FlowNode {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub position: Position,
    #[serde(default)]
    pub config: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FlowEdge {
    pub id: String,
    pub source: String,
    pub source_port: String,
    pub target: String,
    pub target_port: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationIssue {
    pub severity: String,
    pub path: String,
    pub message: String,
}

pub fn safe_identifier(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id != "."
        && id != ".."
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_- .".contains(c))
        && !id.contains(' ')
}

pub fn valid_message_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 512
        && path.split('.').all(|p| {
            !p.is_empty()
                && !["__proto__", "prototype", "constructor"].contains(&p)
                && p.chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        })
}

pub fn validate_project(project: &Project) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    let mut issue = |path: String, message: &str| {
        issues.push(ValidationIssue {
            severity: "error".into(),
            path,
            message: message.into(),
        })
    };
    if ![1, 2].contains(&project.schema_version) {
        issue("schemaVersion".into(), "不支持此项目格式版本");
    }
    if !safe_identifier(&project.id) {
        issue("id".into(), "项目 ID 无效");
    }
    if project.name.trim().is_empty() {
        issue("name".into(), "项目名称不能为空");
    }
    if !project.width.is_finite()
        || !project.height.is_finite()
        || project.width <= 0.0
        || project.height <= 0.0
    {
        issue("width".into(), "画布尺寸必须为有限正数");
    }
    if project.pages.is_empty() {
        issue("pages".into(), "至少需要一个页面");
    }
    let mut all_ids = HashSet::new();
    let mut check_id = |id: &str, path: String| {
        if !safe_identifier(id) || !all_ids.insert(id.to_owned()) {
            issue(path, "ID 无效或重复");
        }
    };
    for (i, p) in project.pages.iter().enumerate() {
        check_id(&p.id, format!("pages[{i}].id"));
        for (j, w) in p.widgets.iter().enumerate() {
            check_id(&w.id, format!("pages[{i}].widgets[{j}].id"));
        }
    }
    for (i, c) in project.connections.iter().enumerate() {
        check_id(&c.id, format!("connections[{i}].id"));
    }
    for (i, v) in project.variables.iter().enumerate() {
        check_id(&v.id, format!("variables[{i}].id"));
    }
    for (i, f) in project.flows.iter().enumerate() {
        check_id(&f.id, format!("flows[{i}].id"));
    }
    for (i, action) in project.actions.iter().enumerate() {
        check_id(&action.id, format!("actions[{i}].id"));
    }
    drop(check_id);
    let vars: HashSet<&str> = project.variables.iter().map(|v| v.id.as_str()).collect();
    let connections: HashSet<&str> = project.connections.iter().map(|v| v.id.as_str()).collect();
    let pages: HashSet<&str> = project.pages.iter().map(|v| v.id.as_str()).collect();
    let widgets: HashSet<&str> = project
        .pages
        .iter()
        .flat_map(|p| &p.widgets)
        .map(|w| w.id.as_str())
        .collect();
    let widget_by_id: HashMap<&str, (&Page, &Widget)> = project
        .pages
        .iter()
        .flat_map(|page| {
            page.widgets
                .iter()
                .map(move |widget| (widget.id.as_str(), (page, widget)))
        })
        .collect();
    let flows: HashMap<&str, &Flow> = project.flows.iter().map(|v| (v.id.as_str(), v)).collect();
    let valid_point = |key: &str| {
        key.split_once('.')
            .is_some_and(|(c, p)| connections.contains(c) && !p.is_empty())
    };
    let shared_action_ids: HashSet<&str> = project
        .actions
        .iter()
        .map(|action| action.id.as_str())
        .collect();
    let check_action = |action: &UiAction| {
        if action.kind == "shared" {
            return action
                .action_id
                .as_deref()
                .is_some_and(|id| shared_action_ids.contains(id));
        }
        flows.get(action.flow_id.as_str()).is_some_and(|flow| {
            if action.kind == "flow" {
                match &action.entry_node_id {
                    Some(id) => flow
                        .nodes
                        .iter()
                        .any(|node| &node.id == id && node.kind == "trigger.ui"),
                    None => {
                        flow.nodes
                            .iter()
                            .filter(|node| node.kind == "trigger.ui")
                            .count()
                            == 1
                    }
                }
            } else {
                ["startFlow", "stopFlow"].contains(&action.kind.as_str())
            }
        })
    };
    let check_ref = |value: &ValueRef| match value {
        ValueRef::Literal { .. } => true,
        ValueRef::Message { path } => valid_message_path(path),
        ValueRef::Variable { key } => vars.contains(key.as_str()),
        ValueRef::Input { key } => widgets.contains(key.as_str()),
        ValueRef::Point { key } => valid_point(key),
        ValueRef::Property { key, property } => {
            valid_message_path(property)
                && widget_by_id.get(key.as_str()).is_some_and(|(_, widget)| {
                    widget.props.contains_key(property)
                        || widget.custom_properties.contains_key(property)
                })
        }
    };
    for (i, c) in project.connections.iter().enumerate() {
        if c.port.trim().is_empty()
            || c.baud_rate == 0
            || !(5..=8).contains(&c.data_bits)
            || ![1, 2].contains(&c.stop_bits)
            || !["none", "odd", "even"].contains(&c.parity.as_str())
            || !safe_identifier(&c.protocol_id)
        {
            issue(format!("connections[{i}]"), "串口参数或协议 ID 无效");
        }
        if c.port.starts_with("sim://") && c.port != "sim://temperature" {
            issue(format!("connections[{i}].port"), "未知模拟端口");
        }
    }
    for (i, v) in project.variables.iter().enumerate() {
        if !v.accepts(&v.initial) {
            issue(format!("variables[{i}].initial"), "变量初值不符合声明类型");
        }
    }
    for (i, p) in project.pages.iter().enumerate() {
        for (j, w) in p.widgets.iter().enumerate() {
            let path = format!("pages[{i}].widgets[{j}]");
            if ![
                "text",
                "button",
                "input",
                "number",
                "select",
                "switch",
                "indicator",
                "group",
                "chart",
                "log",
                "tool-button",
                "textarea",
                "checkbox",
                "radio",
                "spinbox",
                "date",
                "time",
                "datetime",
                "slider",
                "image",
                "progress",
                "separator",
                "list",
                "tree",
                "table",
                "frame",
                "scroll",
                "tabs",
                "stack",
                "splitter",
                "dialog",
                "menu",
                "toolbar",
                "statusbar",
                "spacer",
            ]
            .contains(&w.kind.as_str())
            {
                issue(format!("{path}.kind"), "不支持此控件类型");
            }
            if ![w.x, w.y, w.width, w.height].iter().all(|v| v.is_finite())
                || w.width <= 0.0
                || w.height <= 0.0
            {
                issue(path.clone(), "控件位置或尺寸无效");
            }
            if let Some(parent_id) = &w.parent_id {
                match widget_by_id.get(parent_id.as_str()) {
                    Some((parent_page, parent)) if parent_page.id == p.id => {
                        if let Some(slot_id) = &w.slot_id {
                            if !parent.slots.iter().any(|slot| slot.id == *slot_id) {
                                issue(format!("{path}.slotId"), "容器槽位不存在");
                            }
                        }
                    }
                    _ => issue(format!("{path}.parentId"), "父容器不存在或不属于当前页面"),
                }
                let mut ancestor = Some(parent_id.as_str());
                let mut visited = HashSet::new();
                while let Some(id) = ancestor {
                    if id == w.id || !visited.insert(id) {
                        issue(format!("{path}.parentId"), "容器层级不能形成循环");
                        break;
                    }
                    ancestor = widget_by_id
                        .get(id)
                        .and_then(|(_, item)| item.parent_id.as_deref());
                }
            } else if w.slot_id.is_some() {
                issue(format!("{path}.slotId"), "未放入容器的控件不能指定槽位");
            }
            if let Some(layout) = &w.layout {
                if !["absolute", "horizontal", "vertical", "grid", "form"]
                    .contains(&layout.kind.as_str())
                    || layout.gap.is_some_and(|v| !v.is_finite() || v < 0.0)
                    || layout.padding.is_some_and(|v| !v.is_finite() || v < 0.0)
                    || layout.columns.is_some_and(|v| v == 0)
                {
                    issue(format!("{path}.layout"), "布局参数无效");
                }
            }
            for (key, property) in &w.custom_properties {
                let valid = match property.value_type.as_str() {
                    "string" => property.value.is_string(),
                    "number" => property.value.is_number(),
                    "boolean" => property.value.is_boolean(),
                    "enum" => property
                        .value
                        .as_str()
                        .is_some_and(|v| property.options.iter().any(|o| o == v)),
                    "object" => property.value.is_object(),
                    "array" => property.value.is_array(),
                    _ => false,
                };
                if key.is_empty()
                    || !valid
                    || property
                        .binding
                        .as_ref()
                        .is_some_and(|value| !check_ref(value))
                {
                    issue(
                        format!("{path}.customProperties.{key}"),
                        "自定义属性类型、初值或绑定无效",
                    );
                }
            }
            for (key, r) in &w.bindings {
                if !check_ref(r) || matches!(r, ValueRef::Message { .. }) {
                    issue(format!("{path}.bindings.{key}"), "无效绑定引用");
                }
            }
            for (key, a) in &w.events {
                if !check_action(a) {
                    issue(
                        format!("{path}.events.{key}"),
                        "流程事件引用无效或入口不唯一",
                    );
                }
            }
            for (key, custom) in &w.custom_properties {
                if custom
                    .event
                    .as_ref()
                    .is_some_and(|action| action.kind != "flow" || !check_action(action))
                {
                    issue(
                        format!("{path}.customProperties.{key}.event"),
                        "自定义属性事件必须引用有效流程入口",
                    );
                }
            }
        }
    }
    for (i, action) in project.actions.iter().enumerate() {
        if action.action.kind == "shared" || !check_action(&action.action) {
            issue(
                format!("actions[{i}].action"),
                "共享动作必须直接引用有效流程或管理操作",
            );
        }
    }
    let mut property_edges: HashMap<(String, String), (String, String)> = HashMap::new();
    for page in &project.pages {
        for widget in &page.widgets {
            for (property, reference) in &widget.bindings {
                if let ValueRef::Property {
                    key,
                    property: target_property,
                } = reference
                {
                    property_edges.insert(
                        (widget.id.clone(), property.clone()),
                        (key.clone(), target_property.clone()),
                    );
                }
            }
            for (property, custom) in &widget.custom_properties {
                if let Some(ValueRef::Property {
                    key,
                    property: target_property,
                }) = &custom.binding
                {
                    property_edges.insert(
                        (widget.id.clone(), property.clone()),
                        (key.clone(), target_property.clone()),
                    );
                }
            }
        }
    }
    fn property_cycle(
        node: &(String, String),
        edges: &HashMap<(String, String), (String, String)>,
        active: &mut HashSet<(String, String)>,
        done: &mut HashSet<(String, String)>,
    ) -> bool {
        if done.contains(node) {
            return false;
        }
        if !active.insert(node.clone()) {
            return true;
        }
        let cycle = edges
            .get(node)
            .is_some_and(|next| property_cycle(next, edges, active, done));
        active.remove(node);
        done.insert(node.clone());
        cycle
    }
    let mut active = HashSet::new();
    let mut done = HashSet::new();
    if property_edges
        .keys()
        .any(|key| property_cycle(key, &property_edges, &mut active, &mut done))
    {
        issue("bindings".into(), "组件属性绑定不能形成循环引用");
    }
    for (i, f) in project.flows.iter().enumerate() {
        let fp = format!("flows[{i}]");
        if f.timeout_ms == 0 || f.timeout_ms > 86_400_000 {
            issue(
                format!("{fp}.timeoutMs"),
                "运行时限必须在 1 毫秒至 24 小时内",
            );
        }
        let mut nodes = HashMap::new();
        for (j, n) in f.nodes.iter().enumerate() {
            let np = format!("{fp}.nodes[{j}]");
            if !safe_identifier(&n.id) || nodes.insert(n.id.as_str(), n).is_some() {
                issue(format!("{np}.id"), "节点 ID 无效或重复");
            }
            let text = |key: &str| n.config.get(key).and_then(Value::as_str);
            let number = |key: &str| n.config.get(key).and_then(Value::as_u64);
            let reference = |key: &str| {
                n.config
                    .get(key)
                    .cloned()
                    .and_then(|v| serde_json::from_value::<ValueRef>(v).ok())
                    .is_some_and(|r| check_ref(&r))
            };
            let reference_value = |key: &str| {
                n.config
                    .get(key)
                    .cloned()
                    .and_then(|value| serde_json::from_value::<ValueRef>(value).ok())
            };
            let valid = match n.kind.as_str() {
                "trigger.ui" => true,
                "trigger.timer" => {
                    number("intervalMs").is_some_and(|v| (100..=86_400_000).contains(&v))
                }
                "trigger.data" => text("key").is_some_and(valid_point),
                "connection.open" | "connection.close" => {
                    text("connectionId").is_some_and(|id| connections.contains(id))
                }
                "protocol.command" => {
                    text("connectionId").is_some_and(|id| connections.contains(id))
                        && text("command").is_some_and(|v| !v.trim().is_empty())
                        && n.config.get("params").map_or(true, |v| {
                            v.as_object().is_some_and(|m| {
                                m.values().all(|v| {
                                    serde_json::from_value::<ValueRef>(v.clone())
                                        .ok()
                                        .is_some_and(|r| check_ref(&r))
                                })
                            })
                        })
                }
                "condition" => {
                    reference("left")
                        && reference("right")
                        && text("operator").is_some_and(|v| {
                            ["eq", "ne", "gt", "gte", "lt", "lte", "contains"].contains(&v)
                        })
                }
                "assign" => text("path").is_some_and(valid_message_path) && reference("value"),
                "variable.get" => {
                    text("variableId").is_some_and(|id| vars.contains(id))
                        && text("path").map_or(true, valid_message_path)
                }
                "variable.set" => {
                    text("variableId").is_some_and(|id| vars.contains(id)) && reference("value")
                }
                "delay" => number("durationMs").is_some_and(|v| v <= 86_400_000),
                "python" => {
                    text("code").is_some_and(|v| !v.trim().is_empty() && v.len() <= 1024 * 1024)
                        && number("timeoutMs").map_or(!n.config.contains_key("timeoutMs"), |v| {
                            (1..=86_400_000).contains(&v)
                        })
                        && number("memoryMb").map_or(!n.config.contains_key("memoryMb"), |v| {
                            (32..=16_384).contains(&v)
                        })
                }
                "ui.setProperty" => {
                    let widget_id = text("widgetId");
                    let property = text("property");
                    let target =
                        widget_id.and_then(|id| widget_by_id.get(id).map(|(_, widget)| *widget));
                    let descriptor = property.and_then(|key| {
                        target.and_then(|widget| widget.custom_properties.get(key))
                    });
                    let is_bound = property.is_some_and(|key| {
                        target.is_some_and(|widget| {
                            widget.bindings.contains_key(key)
                                || widget
                                    .custom_properties
                                    .get(key)
                                    .is_some_and(|custom| custom.binding.is_some())
                        })
                    });
                    let exists = property.is_some_and(|key| {
                        target.is_some_and(|widget| {
                            widget.props.contains_key(key)
                                || widget.custom_properties.contains_key(key)
                        })
                    });
                    let reference = reference_value("value");
                    let hint = reference.as_ref().and_then(|reference| match reference {
                        ValueRef::Literal { value } => Some(value.clone()),
                        ValueRef::Variable { key } => project
                            .variables
                            .iter()
                            .find(|variable| variable.id == *key)
                            .map(|variable| variable.initial.clone()),
                        ValueRef::Property { key, property } => {
                            widget_by_id.get(key.as_str()).and_then(|(_, widget)| {
                                widget
                                    .custom_properties
                                    .get(property)
                                    .map(|custom| custom.value.clone())
                                    .or_else(|| widget.props.get(property).cloned())
                            })
                        }
                        _ => None,
                    });
                    let type_valid = match (descriptor, hint.as_ref()) {
                        (Some(custom), Some(value)) => match custom.value_type.as_str() {
                            "string" => value.is_string(),
                            "number" => value.is_number(),
                            "boolean" => value.is_boolean(),
                            "enum" => value
                                .as_str()
                                .is_some_and(|v| custom.options.iter().any(|option| option == v)),
                            "object" => value.is_object(),
                            "array" => value.is_array(),
                            _ => false,
                        },
                        _ => true,
                    };
                    text("widgetId").is_some_and(|id| widgets.contains(id))
                        && property.is_some_and(valid_message_path)
                        && exists
                        && !is_bound
                        && reference.as_ref().is_some_and(check_ref)
                        && type_valid
                }
                "page.navigate" => text("pageId").is_some_and(|id| pages.contains(id)),
                "log" => {
                    reference("value")
                        && text("level").map_or(true, |v| ["info", "warn", "error"].contains(&v))
                }
                _ => false,
            };
            if !valid {
                issue(format!("{np}.config"), "节点类型、参数或资源引用无效");
            }
        }
        let mut ids = HashSet::new();
        let mut indegrees: HashMap<&str, usize> = nodes.keys().map(|k| (*k, 0)).collect();
        let mut children: HashMap<&str, Vec<&str>> = HashMap::new();
        for (j, e) in f.edges.iter().enumerate() {
            let ep = format!("{fp}.edges[{j}]");
            if !safe_identifier(&e.id) || !ids.insert(&e.id) {
                issue(format!("{ep}.id"), "连线 ID 无效或重复");
            }
            match (nodes.get(e.source.as_str()), nodes.get(e.target.as_str())) {
                (Some(source), Some(target)) => {
                    let valid_port = if source.kind == "condition" {
                        ["true", "false", "error"].contains(&e.source_port.as_str())
                    } else if source.kind.starts_with("trigger.") {
                        e.source_port == "out"
                    } else {
                        ["out", "error"].contains(&e.source_port.as_str())
                    };
                    if !valid_port || e.target_port != "in" || target.kind.starts_with("trigger.") {
                        issue(ep, "连线端口无效，触发节点不能作为目标");
                    }
                    *indegrees.get_mut(e.target.as_str()).unwrap() += 1;
                    children
                        .entry(e.source.as_str())
                        .or_default()
                        .push(e.target.as_str());
                }
                _ => issue(ep, "连线指向不存在的节点"),
            }
        }
        let mut ready: VecDeque<&str> = indegrees
            .iter()
            .filter_map(|(k, v)| (*v == 0).then_some(*k))
            .collect();
        let mut visited = 0;
        while let Some(id) = ready.pop_front() {
            visited += 1;
            for next in children.get(id).into_iter().flatten() {
                let degree = indegrees.get_mut(next).unwrap();
                *degree -= 1;
                if *degree == 0 {
                    ready.push_back(next);
                }
            }
        }
        if visited != nodes.len() {
            issue(format!("{fp}.edges"), "流程必须为有向无环图");
        }
    }
    for (i, a) in project.assets.iter().enumerate() {
        if !safe_identifier(&a.id)
            || a.path.is_empty()
            || a.path.starts_with(['/', '\\'])
            || a.path.contains(':')
            || a.path.split(['/', '\\']).any(|p| p == "..")
        {
            issue(format!("assets[{i}]"), "资源必须使用项目内相对路径");
        }
    }
    if project
        .flows
        .iter()
        .flat_map(|f| &f.nodes)
        .any(|n| n.kind == "python")
        && project.python.is_none()
    {
        issue("python".into(), "Python 节点需要指定运行资源");
    }
    issues
}

#[cfg(test)]
mod schema_v2_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn v1_defaults_and_v2_layout_i18n_dynamic_properties_round_trip() {
        let legacy: Project = serde_json::from_value(json!({
            "schemaVersion":1,"id":"legacy","name":"Legacy","width":800,"height":600,
            "pages":[{"id":"page","name":"Page","widgets":[{"id":"root","kind":"group","name":"Root","x":0,"y":0,"width":400,"height":300,"zIndex":0,"props":{},"bindings":{},"events":{}}]}]
        })).unwrap();
        assert!(legacy.i18n.is_none());
        assert!(legacy.pages[0].widgets[0].layout.is_none());

        let current: Project = serde_json::from_value(json!({
            "schemaVersion":2,"id":"current","name":"Current","width":800,"height":600,
            "pages":[{"id":"page","name":"Page","layout":{"type":"absolute"},"guides":[],"widgets":[
                {"id":"root","kind":"frame","name":"Root","x":0,"y":0,"width":400,"height":300,"zIndex":0,"props":{},"bindings":{},"events":{},"slots":[{"id":"content","title":"Content"}],"customProperties":{"title":{"type":"string","value":"Hello"}}},
                {"id":"child","kind":"text","name":"Child","x":12,"y":12,"width":120,"height":32,"zIndex":1,"parentId":"root","slotId":"content","props":{},"bindings":{"text":{"source":"property","key":"root","property":"title"}},"events":{},"customProperties":{"title":{"type":"string","value":"Hello"}}}
            ]}],
            "variables":[{"id":"items","name":"Items","type":"array","initial":[]},{"id":"settings","name":"Settings","type":"object","initial":{}}],
            "i18n":{"defaultLocale":"zh-CN","locales":["zh-CN","en-US"],"messages":{"en-US":{"hello":"Hello"}}},"actions":[]
        })).unwrap();
        assert!(
            validate_project(&current)
                .iter()
                .all(|i| i.severity != "error"),
            "{:?}",
            validate_project(&current)
        );
        let round_trip: Project =
            serde_json::from_value(serde_json::to_value(current).unwrap()).unwrap();
        assert_eq!(round_trip.schema_version, 2);
        assert_eq!(round_trip.variables[0].value_type, "array");
    }

    #[test]
    fn rejects_container_cycles_and_unknown_catalog_kinds() {
        let project: Project = serde_json::from_value(json!({
            "schemaVersion":2,"id":"p","name":"P","width":800,"height":600,"pages":[{"id":"page","name":"Page","widgets":[
                {"id":"a","kind":"frame","name":"A","x":0,"y":0,"width":200,"height":200,"zIndex":0,"parentId":"b"},
                {"id":"b","kind":"frame","name":"B","x":0,"y":0,"width":100,"height":100,"zIndex":0,"parentId":"a"},
                {"id":"c","kind":"unknown","name":"C","x":0,"y":0,"width":10,"height":10,"zIndex":1}
            ]}]})).unwrap();
        let issues = validate_project(&project);
        assert!(issues.iter().any(|i| i.path.ends_with("parentId")));
        assert!(issues.iter().any(|i| i.path.ends_with(".kind")));
    }

    #[test]
    fn validates_property_reference_names_and_rejects_binding_cycles() {
        let project: Project = serde_json::from_value(json!({
            "schemaVersion":2,"id":"p","name":"P","width":800,"height":600,
            "pages":[{"id":"page","name":"Page","widgets":[
                {"id":"a","kind":"text","name":"A","x":0,"y":0,"width":50,"height":20,"zIndex":0,"props":{},"bindings":{"text":{"source":"property","key":"b","property":"beta"}},"customProperties":{"alpha":{"type":"string","value":"a","binding":{"source":"property","key":"b","property":"beta"}}}},
                {"id":"b","kind":"text","name":"B","x":60,"y":0,"width":50,"height":20,"zIndex":0,"props":{},"bindings":{},"customProperties":{"beta":{"type":"array","value":[],"binding":{"source":"property","key":"a","property":"alpha"}}}}
            ]}]
        })).unwrap();
        let issues = validate_project(&project);
        assert!(issues
            .iter()
            .any(|issue| issue.message.contains("循环引用")));

        let mut missing = project;
        missing.pages[0].widgets[0].bindings.insert(
            "text".into(),
            ValueRef::Property {
                key: "b".into(),
                property: "missing".into(),
            },
        );
        assert!(validate_project(&missing)
            .iter()
            .any(|issue| issue.path.ends_with("bindings.text")));
    }

    #[test]
    fn ui_set_property_requires_an_existing_unbound_property_and_matching_type() {
        let project: Project = serde_json::from_value(json!({
            "schemaVersion":2,"id":"p","name":"P","width":800,"height":600,
            "pages":[{"id":"page","name":"Page","widgets":[{"id":"w","kind":"text","name":"W","x":0,"y":0,"width":80,"height":24,"zIndex":0,"props":{},"customProperties":{"level":{"type":"number","value":0}}}]}],
            "flows":[{"id":"f","name":"F","enabled":true,"timeoutMs":1000,"nodes":[{"id":"set","kind":"ui.setProperty","label":"Set","position":{"x":0,"y":0},"config":{"widgetId":"w","property":"level","value":{"source":"literal","value":5}}}],"edges":[]}]
        })).unwrap();
        assert!(validate_project(&project)
            .iter()
            .all(|issue| issue.severity != "error"));

        let mut wrong_type = project.clone();
        wrong_type.flows[0].nodes[0]
            .config
            .insert("value".into(), json!({"source":"literal","value":"bad"}));
        assert!(validate_project(&wrong_type)
            .iter()
            .any(|issue| issue.path.ends_with("nodes[0].config")));

        let mut bound = project;
        bound.pages[0].widgets[0]
            .bindings
            .insert("level".into(), ValueRef::Literal { value: json!(1) });
        assert!(validate_project(&bound)
            .iter()
            .any(|issue| issue.path.ends_with("nodes[0].config")));
    }
}
