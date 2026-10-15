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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "camelCase", deny_unknown_fields)]
pub enum ValueRef {
    Literal { value: Value },
    Message { path: String },
    Variable { key: String },
    Input { key: String },
    Point { key: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiAction {
    pub kind: String,
    pub flow_id: String,
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
    if project.schema_version != 1 {
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
    let flows: HashMap<&str, &Flow> = project.flows.iter().map(|v| (v.id.as_str(), v)).collect();
    let valid_point = |key: &str| {
        key.split_once('.')
            .is_some_and(|(c, p)| connections.contains(c) && !p.is_empty())
    };
    let check_ref = |value: &ValueRef| match value {
        ValueRef::Literal { .. } => true,
        ValueRef::Message { path } => valid_message_path(path),
        ValueRef::Variable { key } => vars.contains(key.as_str()),
        ValueRef::Input { key } => widgets.contains(key.as_str()),
        ValueRef::Point { key } => valid_point(key),
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
            for (key, r) in &w.bindings {
                if !check_ref(r) || matches!(r, ValueRef::Message { .. }) {
                    issue(format!("{path}.bindings.{key}"), "无效绑定引用");
                }
            }
            for (key, a) in &w.events {
                let valid = flows.get(a.flow_id.as_str()).is_some_and(|f| {
                    if a.kind == "flow" {
                        match &a.entry_node_id {
                            Some(id) => f
                                .nodes
                                .iter()
                                .any(|n| &n.id == id && n.kind == "trigger.ui"),
                            None => f.nodes.iter().filter(|n| n.kind == "trigger.ui").count() == 1,
                        }
                    } else {
                        ["startFlow", "stopFlow"].contains(&a.kind.as_str())
                    }
                });
                if !valid {
                    issue(
                        format!("{path}.events.{key}"),
                        "流程事件引用无效或入口不唯一",
                    );
                }
            }
        }
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
