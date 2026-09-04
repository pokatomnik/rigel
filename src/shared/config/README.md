# Configuration

This module owns Rigel's persisted configuration and application-wide file
names. It loads, validates, and writes the user configuration and defines the
default Rigel storage locations.

The global layer is `~/.rigel/config.toml`, unless `chat --profile <PATH>`
selects another global path. The project layer is always
`<current-dir>/.rigel/config.toml`. Scalar values from the project layer
replace global values; maps merge by key with project precedence, and vectors
append project values after global values. The project layer is optional and
does not disable the global layer.

Tool permissions are stored under `[policies."tool_name"]` with an explicit
`allow = true` or `allow = false`. Project policies override global policies.
`rigel init` leaves the policies section empty.
