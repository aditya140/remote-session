# Workspace checks

Run `cargo xtask boundaries` to check direct product dependencies against the component graph. The checker rejects forbidden gateway execution/UI dependencies. Its tests include a forbidden edge.

Run `cargo xtask schema` to export the public protocol schema. Run `cargo xtask schema --check` in CI to reject stale output. Administrative IPC is excluded from the schema.

This developer tool owns no runtime state. It does not launch product services or retry mutations.
