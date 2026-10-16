# Protocol plugins

Protocol plugins let projects add device-specific commands while keeping serial-port ownership and flow execution in the host. A plugin is an independent process that communicates with the host through newline-delimited UTF-8 JSON-RPC 2.0 on standard input and output. Each line is one complete JSON object; protocol output uses standard output only, while diagnostics go to standard error.

The host starts a plugin with its manifest-declared executable and arguments. It sends initialize with {apiVersion, protocolId, config, simulation}. The plugin returns an object containing the supported API version. A project connection selects a protocol ID and may provide a protocol version and JSON configuration.

The plugin can implement these host requests and notifications:

| Method | Direction | Purpose |
| --- | --- | --- |
| initialize | Host to plugin | Negotiate the API version and pass connection configuration. |
| command.execute | Host to plugin | Execute a declared command with its parameters and timeout. |
| transport.data | Host to plugin | Deliver a base64-encoded serial read block. A block may contain a partial frame or multiple frames. |
| transport.write | Plugin to host | Write a base64-encoded byte block, limited to 64 KiB per request. |
| point.publish | Plugin to host | Publish a declared data point, optionally with the originating flow ID. |
| log | Plugin to host | Send a structured diagnostic message. |

The host serializes commands for each connection and caps its pending queue at 64. Stopping a flow cancels requests that have not been sent. Once a request has been sent, the host drains its response or waits for the configured deadline before continuing; it does not retry commands automatically. A plugin must keep reading input while waiting for device responses so it can receive transport.data notifications.

JSON-RPC errors use the standard error object with a stable plugin error code in data.code when available. The host terminates the plugin process when the connection closes. Plugin shutdown must not be required to complete a device operation that the host has already sent.

## Manifest

Each plugin package contains a manifest.json and the executable named by its executable field. The manifest identifies the plugin, its version, API version, target triple, executable, and optional arguments. It declares its configuration schema, commands and their parameter schemas, and published points.

The host validates the manifest and connection configuration before starting the process. Supported schema constraints include JSON types, properties, required fields, additional-property policy, enums, constants, numeric bounds, string lengths, array item schemas, and array length bounds. Unknown constraint keywords are rejected rather than silently ignored. A missing schema means that the corresponding object has no schema-level restrictions.

The target must match the platform-specific plugin executable. Plugin dependencies and runtime files belong inside the plugin package and are distributed with it. The host's optional Python execution environment and its dependencies remain separate.

Plugins should correlate device responses to command requests where the wire protocol permits it. If an old response cannot be distinguished from a later command's response, the plugin should report the connection as faulted after a timeout instead of assigning that response to the next command. The host does not infer device identity or implement a plugin's wire protocol.
