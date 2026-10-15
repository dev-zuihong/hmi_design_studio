use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use hmi_core::{
    framing::{decode_demo_frame, encode_demo_frame, LineDecoder},
    CoreError, CoreResult,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

/// Reference device shared by the explicit simulator and serial device CLI.
pub struct TemperatureDevice {
    temperature: f64,
    target: f64,
}
impl Default for TemperatureDevice {
    fn default() -> Self {
        Self {
            temperature: 23.5,
            target: 25.0,
        }
    }
}
impl TemperatureDevice {
    pub fn execute(&mut self, command: &str, params: &Value) -> CoreResult<Value> {
        if !params.is_object() {
            return Err(CoreError::new("PARAMETERS", "Parameters must be an object"));
        }
        match command {
            "read" => {
                if !params.as_object().unwrap().is_empty() {
                    return Err(CoreError::new("PARAMETERS", "read accepts no parameters"));
                }
                self.temperature =
                    ((self.temperature + (self.target - self.temperature) * 0.08) * 10.0).round()
                        / 10.0;
            }
            "setTarget" => {
                let value = params
                    .get("target")
                    .and_then(Value::as_f64)
                    .filter(|n| n.is_finite() && (-40.0..=200.0).contains(n))
                    .ok_or_else(|| {
                        CoreError::new("PARAMETERS", "target must be between -40 and 200 C")
                    })?;
                if params.as_object().unwrap().len() != 1 {
                    return Err(CoreError::new(
                        "PARAMETERS",
                        "Unexpected setTarget parameter",
                    ));
                }
                self.target = value;
            }
            _ => return Err(CoreError::new("COMMAND_MISSING", "Unknown device command")),
        }
        Ok(
            json!({"temperature":self.temperature,"rawTemperature":(self.temperature*10.0).round() as i64,"target":self.target}),
        )
    }
    pub fn wire_response(&mut self, request: &Value) -> CoreResult<Vec<u8>> {
        let id = request.get("id").and_then(Value::as_u64).ok_or_else(|| {
            CoreError::new("DEVICE_FRAME", "A numeric correlation ID is required")
        })?;
        let result = self.execute(
            request.get("command").and_then(Value::as_str).unwrap_or(""),
            &request.get("params").cloned().unwrap_or(json!({})),
        );
        encode_demo_frame(&match result {
            Ok(value) => json!({"id":id,"result":value}),
            Err(error) => json!({"id":id,"error":error}),
        })
    }
}

struct Pending {
    rpc_id: Value,
    deadline: Instant,
    origin: Value,
    simulation: Option<(Instant, String, Value)>,
}
pub struct ProtocolEngine {
    initialized: bool,
    simulated: bool,
    simulation_delay_ms: u64,
    next_id: u64,
    pending: HashMap<u64, Pending>,
    decoder: LineDecoder,
    device: TemperatureDevice,
}
impl Default for ProtocolEngine {
    fn default() -> Self {
        Self {
            initialized: false,
            simulated: false,
            simulation_delay_ms: 30,
            next_id: 1,
            pending: HashMap::new(),
            decoder: LineDecoder::new(64 * 1024),
            device: TemperatureDevice::default(),
        }
    }
}
fn reply(id: Value, result: CoreResult<Value>) -> Value {
    match result {
        Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
        Err(error) => {
            json!({"jsonrpc":"2.0","id":id,"error":{"code":-32000,"message":error.message,"data":{"code":error.code}}})
        }
    }
}
impl ProtocolEngine {
    pub fn receive(&mut self, message: Value, now: Instant) -> Vec<Value> {
        let mut output = Vec::new();
        if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return output;
        }
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let id = message.get("id").cloned();
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        match method {
            "initialize" => {
                if let Some(id) = id {
                    let config = params.get("config").cloned().unwrap_or(json!({}));
                    let valid = config.as_object().is_some_and(|v| {
                        v.keys()
                            .all(|k| ["timeoutMs", "simulationDelayMs"].contains(&k.as_str()))
                    }) && config.get("timeoutMs").map_or(true, |v| {
                        v.as_u64().is_some_and(|n| (10..=60_000).contains(&n))
                    }) && config
                        .get("simulationDelayMs")
                        .map_or(true, |v| v.as_u64().is_some_and(|n| n <= 30_000));
                    let result = if self.initialized {
                        Err(CoreError::new(
                            "INITIALIZED",
                            "Plugin is already initialized",
                        ))
                    } else if params["apiVersion"] != 1
                        || params["protocolId"] != "demo-temperature"
                    {
                        Err(CoreError::new("PLUGIN_API", "Unsupported protocol/API"))
                    } else if !valid {
                        Err(CoreError::new(
                            "PROTOCOL_CONFIG",
                            "Invalid protocol configuration",
                        ))
                    } else {
                        self.simulated = params
                            .get("simulation")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        self.simulation_delay_ms = config
                            .get("simulationDelayMs")
                            .and_then(Value::as_u64)
                            .unwrap_or(30);
                        self.initialized = true;
                        Ok(
                            json!({"apiVersion":1,"lateResponsesDiscarded":true,"simulation":self.simulated}),
                        )
                    };
                    output.push(reply(id, result));
                }
            }
            "command.execute" => {
                if let Some(id) = id {
                    if !self.initialized {
                        output.push(reply(
                            id,
                            Err(CoreError::new(
                                "NOT_INITIALIZED",
                                "Initialize the plugin first",
                            )),
                        ));
                    } else if !self.pending.is_empty() {
                        output.push(reply(
                            id,
                            Err(CoreError::new(
                                "DEVICE_BUSY",
                                "Only one wire transaction is allowed",
                            )),
                        ));
                    } else {
                        let command = params
                            .get("command")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned();
                        let arguments = params.get("params").cloned().unwrap_or(json!({}));
                        let mut validator = TemperatureDevice::default();
                        if let Err(error) = validator.execute(&command, &arguments) {
                            output.push(reply(id, Err(error)));
                            return output;
                        }
                        let wire_id = self.next_id;
                        self.next_id += 1;
                        let timeout = params
                            .get("timeoutMs")
                            .and_then(Value::as_u64)
                            .unwrap_or(2000)
                            .clamp(10, 60_000);
                        self.pending.insert(
                            wire_id,
                            Pending {
                                rpc_id: id,
                                deadline: now + Duration::from_millis(timeout),
                                origin: params.get("originFlowId").cloned().unwrap_or(Value::Null),
                                simulation: self.simulated.then(|| {
                                    (
                                        now + Duration::from_millis(self.simulation_delay_ms),
                                        command.clone(),
                                        arguments.clone(),
                                    )
                                }),
                            },
                        );
                        if !self.simulated {
                            let bytes = encode_demo_frame(
                                &json!({"id":wire_id,"command":command,"params":arguments}),
                            )
                            .unwrap();
                            output.push(json!({"jsonrpc":"2.0","id":format!("write-{wire_id}"),"method":"transport.write","params":{"bytes":BASE64.encode(bytes)}}));
                        }
                    }
                }
            }
            "transport.data" if self.initialized && !self.simulated => {
                let bytes = params
                    .get("bytes")
                    .and_then(Value::as_str)
                    .and_then(|s| BASE64.decode(s).ok());
                match bytes
                    .filter(|b| b.len() <= 64 * 1024)
                    .ok_or_else(|| CoreError::new("TRANSPORT_BYTES", "Invalid transport data"))
                    .and_then(|b| self.decoder.push(&b))
                {
                    Ok(lines) => {
                        for line in lines {
                            match decode_demo_frame(&line) {
                                Ok(frame) => {
                                    if let Some(wire_id) = frame.get("id").and_then(Value::as_u64) {
                                        let result = if let Some(error) = frame.get("error") {
                                            Err(CoreError::new(
                                                error
                                                    .get("code")
                                                    .and_then(Value::as_str)
                                                    .unwrap_or("DEVICE_ERROR"),
                                                error
                                                    .get("message")
                                                    .and_then(Value::as_str)
                                                    .unwrap_or("Device error"),
                                            ))
                                        } else {
                                            frame.get("result").cloned().ok_or_else(|| {
                                                CoreError::new("DEVICE_FRAME", "Missing result")
                                            })
                                        };
                                        self.finish(wire_id, result, &mut output);
                                    }
                                }
                                Err(error) => self.fail_current(error, &mut output),
                            }
                        }
                    }
                    Err(error) => self.fail_current(error, &mut output),
                }
            }
            "" => {
                if let Some(wire_id) = id
                    .as_ref()
                    .and_then(Value::as_str)
                    .and_then(|s| s.strip_prefix("write-"))
                    .and_then(|s| s.parse::<u64>().ok())
                {
                    if let Some(error) = message.get("error") {
                        self.finish(
                            wire_id,
                            Err(CoreError::new(
                                "SERIAL_WRITE",
                                error
                                    .get("message")
                                    .and_then(Value::as_str)
                                    .unwrap_or("Transport write failed"),
                            )),
                            &mut output,
                        );
                    }
                }
            }
            _ => {
                if let Some(id) = id {
                    output.push(reply(
                        id,
                        Err(CoreError::new(
                            "METHOD_MISSING",
                            "Unsupported plugin method",
                        )),
                    ));
                }
            }
        }
        output
    }
    pub fn tick(&mut self, now: Instant) -> Vec<Value> {
        let mut output = Vec::new();
        let ready: Vec<_> = self
            .pending
            .iter()
            .filter_map(|(id, p)| {
                if now >= p.deadline {
                    Some((*id, true))
                } else if p.simulation.as_ref().is_some_and(|(due, _, _)| now >= *due) {
                    Some((*id, false))
                } else {
                    None
                }
            })
            .collect();
        for (id, expired) in ready {
            let result = if expired {
                Err(CoreError::new(
                    "DEVICE_TIMEOUT",
                    "Device response deadline exceeded",
                ))
            } else {
                let (_, command, args) =
                    self.pending.get(&id).unwrap().simulation.as_ref().unwrap();
                self.device.execute(command, args)
            };
            self.finish(id, result, &mut output);
        }
        output
    }
    fn fail_current(&mut self, error: CoreError, output: &mut Vec<Value>) {
        let ids: Vec<_> = self.pending.keys().copied().collect();
        for id in ids {
            self.finish(id, Err(error.clone()), output);
        }
    }
    fn finish(&mut self, id: u64, result: CoreResult<Value>, output: &mut Vec<Value>) {
        // Unknown or late wire IDs never complete a later request.
        let Some(pending) = self.pending.remove(&id) else {
            return;
        };
        let result = result.and_then(|value| {
            if ["temperature", "rawTemperature", "target"]
                .iter()
                .all(|key| value.get(key).is_some_and(Value::is_number))
            {
                Ok(value)
            } else {
                Err(CoreError::new(
                    "DEVICE_FRAME",
                    "Invalid device result point types",
                ))
            }
        });
        if let Ok(value) = &result {
            for key in ["temperature", "rawTemperature", "target"] {
                output.push(json!({"jsonrpc":"2.0","method":"point.publish","params":{"key":key,"value":value[key],"originFlowId":pending.origin}}));
            }
        }
        output.push(reply(pending.rpc_id, result));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn initialized() -> ProtocolEngine {
        let mut engine = ProtocolEngine::default();
        let result=engine.receive(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"apiVersion":1,"protocolId":"demo-temperature","config":{},"simulation":false}}),Instant::now());
        assert_eq!(result[0]["result"]["lateResponsesDiscarded"], true);
        engine
    }
    fn request(id: u64) -> Value {
        json!({"jsonrpc":"2.0","id":id,"method":"command.execute","params":{"command":"read","params":{},"timeoutMs":20,"originFlowId":"flow"}})
    }
    fn transport(bytes: &[u8]) -> Value {
        json!({"jsonrpc":"2.0","method":"transport.data","params":{"bytes":BASE64.encode(bytes)}})
    }
    #[test]
    fn split_serial_response_and_origin_propagation() {
        let mut engine = initialized();
        let now = Instant::now();
        let outgoing = engine.receive(request(2), now);
        assert_eq!(outgoing[0]["method"], "transport.write");
        let bytes = BASE64
            .decode(outgoing[0]["params"]["bytes"].as_str().unwrap())
            .unwrap();
        let frame = decode_demo_frame(&bytes[..bytes.len() - 1]).unwrap();
        let response = TemperatureDevice::default().wire_response(&frame).unwrap();
        assert!(engine.receive(transport(&response[..3]), now).is_empty());
        let result = engine.receive(transport(&response[3..]), now);
        assert_eq!(result.len(), 4);
        assert_eq!(result[0]["params"]["originFlowId"], "flow");
        assert_eq!(result[3]["id"], 2);
        assert!(result[3]["result"]["temperature"].is_number());
    }
    #[test]
    fn timed_out_reply_cannot_satisfy_next_command() {
        let mut engine = initialized();
        let now = Instant::now();
        engine.receive(request(2), now);
        let timeout = engine.tick(now + Duration::from_millis(21));
        assert_eq!(timeout[0]["error"]["data"]["code"], "DEVICE_TIMEOUT");
        engine.receive(request(3), now + Duration::from_millis(22));
        let old = encode_demo_frame(
            &json!({"id":1,"result":{"temperature":99,"rawTemperature":990,"target":25}}),
        )
        .unwrap();
        assert!(engine
            .receive(transport(&old), now + Duration::from_millis(23))
            .is_empty());
        let fresh = encode_demo_frame(
            &json!({"id":2,"result":{"temperature":24,"rawTemperature":240,"target":25}}),
        )
        .unwrap();
        let result = engine.receive(transport(&fresh), now + Duration::from_millis(24));
        assert_eq!(result.last().unwrap()["id"], 3);
        assert_eq!(result.last().unwrap()["result"]["temperature"], 24);
    }
}
