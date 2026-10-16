pub mod connection;
pub mod error;
pub mod framing;
pub mod model;
pub mod protocol;
pub mod runtime;
mod schema;
pub use connection::{list_serial_ports, ConnectionManager, SerialPortInfo};
pub use error::{CoreError, CoreResult};
pub use model::*;
pub use protocol::{
    discover_plugins, host_target, load_plugin_manifest, CommandDescriptor, PluginManifest,
    PointDescriptor,
};
pub use runtime::{HostServices, Runtime, RuntimeEvent, RuntimeSnapshot};
